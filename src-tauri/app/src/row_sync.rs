//! Row by row reconciliation of the library database between a «Con copia»
//! client and its host (`host_mirror`, `host_sync`).
//!
//! Every library database keeps a log of which row of which table changed
//! last and when (`notia_row_changes`), written by triggers that
//! [`ensure_tracking`] creates after each migration. The log never keeps the
//! row itself: whoever reads the log reads the row as it is now.
//!
//! - **Host:** every change gets a growing `seq`, so a copy asks for what
//!   changed after the last one it took ([`changes_since`]).
//! - **Copy:** the rows changed on this device are `pending` until the host
//!   takes them ([`pending_changes`], [`mark_sent`]). Rows taken from the
//!   host are recorded as not pending.
//! - **Conflicts** ([`apply_changes`]): a change replaces the row only when
//!   it is newer than the last change this database knows for that row; a
//!   tie keeps the row as it is. Times come from each device's clock.
//!
//! Rows are applied with foreign keys off: the other side already applied
//! its cascades, and they travel as their own changes.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Tables whose rows never travel: the log itself, the schema version, the
/// file index each device builds, the per-device operation journal and the
/// lease of scheduled actions (renewed every minute by the host).
const UNTRACKED: &[&str] = &[
    "notia_schema_migrations",
    "notia_row_changes",
    "notia_sync_meta",
    "library_inventory",
    "library_inventory_staging",
    "library_inventory_state",
    "backend_operation_journal",
    "ai_action_meta",
];
/// Version of the triggers: another body gets other names, so older
/// triggers are replaced.
const TRIGGER_PREFIX: &str = "notia_track_v1_";
const NOW_MS: &str = "CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)";
/// Key of an encoded BLOB value: `{ "$blob": "<base64>" }`.
const BLOB_KEY: &str = "$blob";

const SETUP: &str = "CREATE TABLE IF NOT EXISTS notia_row_changes (
         seq INTEGER PRIMARY KEY AUTOINCREMENT,
         table_name TEXT NOT NULL,
         row_key TEXT NOT NULL,
         changed_ms INTEGER NOT NULL,
         pending INTEGER NOT NULL DEFAULT 1,
         UNIQUE (table_name, row_key)
     );
     CREATE INDEX IF NOT EXISTS idx_notia_row_changes_pending
         ON notia_row_changes(pending) WHERE pending = 1;
     CREATE TABLE IF NOT EXISTS notia_sync_meta (
         key TEXT PRIMARY KEY,
         value TEXT NOT NULL
     );
     INSERT OR IGNORE INTO notia_sync_meta (key, value) VALUES ('database_id', lower(hex(randomblob(16))));";

/// The last change of one row: the row as it is now, or `None` when it was
/// deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RowChange {
    pub(crate) table: String,
    /// Values of the primary key, in its order.
    pub(crate) key: Vec<Value>,
    pub(crate) changed_ms: i64,
    pub(crate) row: Option<Map<String, Value>>,
}

/// Changes of the host after a `seq`, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangeBatch {
    pub(crate) changes: Vec<RowChange>,
    /// The `seq` to ask from next time.
    pub(crate) last_seq: i64,
    /// More changes are waiting after `last_seq`.
    pub(crate) more: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyReport {
    /// Rows replaced or deleted.
    pub(crate) applied: usize,
    /// Rows kept because this side changed them later.
    pub(crate) kept: usize,
    /// Rows that could not be written (an unknown table, a constraint).
    pub(crate) failed: usize,
}

/// Columns and primary key of a tracked table.
#[derive(Debug, Clone)]
struct TableInfo {
    columns: Vec<String>,
    key: Vec<String>,
}

fn quoted(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Creates the log and the triggers of the tables that lack them. Cheap
/// when everything is in place: two reads of the schema.
pub(crate) fn ensure_tracking(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(SETUP)?;
    let triggers: HashSet<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'notia_track_%'")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for table in table_names(connection)? {
        if trigger_names(&table).iter().all(|name| triggers.contains(name)) {
            continue;
        }
        if let Some(info) = table_info(connection, &table)? {
            connection.execute_batch(&triggers_sql(&table, &info.key))?;
        }
    }
    Ok(())
}

/// Tables of the library whose rows may travel.
fn table_names(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let names: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(names.into_iter().filter(|name| !UNTRACKED.contains(&name.as_str())).collect())
}

fn trigger_names(table: &str) -> [String; 3] {
    ["insert", "update", "delete"].map(|event| format!("{TRIGGER_PREFIX}{table}_{event}"))
}

/// Columns and key of a table whose rows may travel: it has a primary key
/// of text or integer columns (`json_array` cannot hold a BLOB, and a
/// failing trigger would fail the app's own write).
fn table_info(connection: &Connection, table: &str) -> rusqlite::Result<Option<TableInfo>> {
    if UNTRACKED.contains(&table) {
        return Ok(None);
    }
    let mut columns = Vec::new();
    let mut key: Vec<(i64, String, String)> = Vec::new();
    let mut statement = connection.prepare("SELECT name, upper(type), pk FROM pragma_table_info(?1)")?;
    let mut rows = statement.query([table])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let declared: String = row.get(1)?;
        let position: i64 = row.get(2)?;
        if position > 0 {
            key.push((position, name.clone(), declared));
        }
        columns.push(name);
    }
    key.sort();
    let trackable = |declared: &str| ["INT", "TEXT", "CHAR", "CLOB"].iter().any(|kind| declared.contains(kind));
    if key.is_empty() || key.iter().any(|(_, _, declared)| !trackable(declared)) {
        return Ok(None);
    }
    Ok(Some(TableInfo { columns, key: key.into_iter().map(|(_, name, _)| name).collect() }))
}

fn triggers_sql(table: &str, key: &[String]) -> String {
    let [insert, update, delete] = trigger_names(table).map(|name| quoted(&name));
    let row_key = |alias: &str| {
        let values: Vec<String> = key.iter().map(|column| format!("{alias}.{}", quoted(column))).collect();
        format!("json_array({})", values.join(", "))
    };
    let (new, old) = (row_key("NEW"), row_key("OLD"));
    let name = literal(table);
    let target = quoted(table);
    // The entry is deleted and inserted again (a new `seq`). Not `INSERT OR
    // REPLACE`: inside a trigger, the conflict policy of the app's own
    // statement (an upsert, `OR IGNORE`) replaces the trigger's.
    let log = |row_key: &str, when: &str| {
        format!(
            "DELETE FROM notia_row_changes WHERE table_name = {name} AND row_key = {row_key}{when};
             INSERT INTO notia_row_changes (table_name, row_key, changed_ms) SELECT {name}, {row_key}, {NOW_MS} WHERE 1{when};"
        )
    };
    let key_changed = format!(" AND {old} IS NOT {new}");
    format!(
        "DROP TRIGGER IF EXISTS {insert}; DROP TRIGGER IF EXISTS {update}; DROP TRIGGER IF EXISTS {delete};
         CREATE TRIGGER {insert} AFTER INSERT ON {target} BEGIN {insert_log} END;
         CREATE TRIGGER {update} AFTER UPDATE ON {target} BEGIN {old_log} {update_log} END;
         CREATE TRIGGER {delete} AFTER DELETE ON {target} BEGIN {delete_log} END;",
        insert_log = log(&new, ""),
        old_log = log(&old, &key_changed),
        update_log = log(&new, ""),
        delete_log = log(&old, ""),
    )
}

/// The id of this database, kept by its copies: another id means the host
/// replaced its database.
pub(crate) fn database_id(connection: &Connection) -> rusqlite::Result<Option<String>> {
    connection
        .query_row("SELECT value FROM notia_sync_meta WHERE key='database_id'", [], |row| row.get(0))
        .optional()
        .or_else(|error| if missing_table(&error) { Ok(None) } else { Err(error) })
}

/// The last `seq` given to a change (0 without changes).
pub(crate) fn last_seq(connection: &Connection) -> rusqlite::Result<i64> {
    connection
        .query_row("SELECT seq FROM sqlite_sequence WHERE name='notia_row_changes'", [], |row| row.get(0))
        .optional()
        .map(Option::unwrap_or_default)
        .or_else(|error| if missing_table(&error) { Ok(0) } else { Err(error) })
}

fn missing_table(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(message)) if message.starts_with("no such table"))
}

// ---------- Values ----------

fn to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::from(number),
        ValueRef::Real(number) => serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => {
            let mut blob = Map::new();
            blob.insert(BLOB_KEY.into(), Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)));
            Value::Object(blob)
        }
    }
}

fn from_json(value: &Value) -> Option<SqlValue> {
    Some(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => SqlValue::Integer(integer),
            None => SqlValue::Real(number.as_f64()?),
        },
        Value::String(text) => SqlValue::Text(text.clone()),
        Value::Object(object) => {
            let encoded = object.get(BLOB_KEY)?.as_str()?;
            SqlValue::Blob(base64::engine::general_purpose::STANDARD.decode(encoded).ok()?)
        }
        Value::Array(_) => return None,
    })
}

/// `json_array(?1, …)` with as many values as the key has, starting at `from`.
fn key_expression(count: usize, from: usize) -> String {
    let placeholders: Vec<String> = (0..count).map(|index| format!("?{}", from + index)).collect();
    format!("json_array({})", placeholders.join(", "))
}

fn key_filter(info: &TableInfo) -> String {
    info.key
        .iter()
        .enumerate()
        .map(|(index, column)| format!("{} IS ?{}", quoted(column), index + 1))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn read_row(connection: &Connection, table: &str, info: &TableInfo, key: &[SqlValue]) -> rusqlite::Result<Option<Map<String, Value>>> {
    let columns: Vec<String> = info.columns.iter().map(|column| quoted(column)).collect();
    let sql = format!("SELECT {} FROM {} WHERE {}", columns.join(", "), quoted(table), key_filter(info));
    connection
        .query_row(&sql, params_from_iter(key.iter()), |row| {
            let mut values = Map::new();
            for (index, column) in info.columns.iter().enumerate() {
                values.insert(column.clone(), to_json(row.get_ref(index)?));
            }
            Ok(values)
        })
        .optional()
}

/// Tables of this database, read once per call.
struct Tables<'a> {
    connection: &'a Connection,
    known: HashMap<String, Option<TableInfo>>,
    names: Option<HashSet<String>>,
}

impl<'a> Tables<'a> {
    fn new(connection: &'a Connection) -> Self {
        Self { connection, known: HashMap::new(), names: None }
    }

    /// A tracked table of this database by its name (never a name built
    /// from what another device sent).
    fn get(&mut self, table: &str) -> rusqlite::Result<Option<&TableInfo>> {
        if self.names.is_none() {
            self.names = Some(table_names(self.connection)?.into_iter().collect());
        }
        if !self.names.as_ref().is_some_and(|names| names.contains(table)) {
            return Ok(None);
        }
        if !self.known.contains_key(table) {
            let info = table_info(self.connection, table)?;
            self.known.insert(table.to_string(), info);
        }
        Ok(self.known.get(table).and_then(Option::as_ref))
    }
}

/// The key of a logged row, when it still fits its table.
fn parse_key(row_key: &str, info: &TableInfo) -> Option<(Vec<Value>, Vec<SqlValue>)> {
    let values: Vec<Value> = serde_json::from_str(row_key).ok()?;
    if values.len() != info.key.len() {
        return None;
    }
    let sql = values.iter().map(from_json).collect::<Option<Vec<_>>>()?;
    Some((values, sql))
}

/// The changes of the rows logged by `query` (`seq`, table, key, time).
fn changes_of(
    connection: &Connection,
    logged: Vec<(i64, String, String, i64)>,
) -> rusqlite::Result<Vec<(i64, Option<RowChange>)>> {
    let mut tables = Tables::new(connection);
    let mut changes = Vec::with_capacity(logged.len());
    for (seq, table, row_key, changed_ms) in logged {
        let Some(info) = tables.get(&table)?.cloned() else {
            changes.push((seq, None));
            continue;
        };
        let Some((key, sql_key)) = parse_key(&row_key, &info) else {
            changes.push((seq, None));
            continue;
        };
        let row = read_row(connection, &table, &info, &sql_key)?;
        changes.push((seq, Some(RowChange { table, key, changed_ms, row })));
    }
    Ok(changes)
}

// ---------- Host ----------

/// Up to `limit` changes after `since`, in the order they happened.
pub(crate) fn changes_since(connection: &Connection, since: i64, limit: usize) -> rusqlite::Result<ChangeBatch> {
    let logged: Vec<(i64, String, String, i64)> = connection
        .prepare("SELECT seq, table_name, row_key, changed_ms FROM notia_row_changes WHERE seq > ?1 ORDER BY seq LIMIT ?2")?
        .query_map(rusqlite::params![since, limit as i64 + 1], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let more = logged.len() > limit;
    let logged: Vec<_> = logged.into_iter().take(limit).collect();
    let last = logged.last().map_or(since, |(seq, ..)| *seq);
    let changes = changes_of(connection, logged)?.into_iter().filter_map(|(_, change)| change).collect();
    Ok(ChangeBatch { changes, last_seq: last, more })
}

// ---------- Copy ----------

/// Up to `limit` rows changed on this device that the host has not taken.
/// Logged rows that no longer fit a table of this database never travel.
pub(crate) fn pending_changes(connection: &Connection, limit: usize) -> rusqlite::Result<Vec<RowChange>> {
    let logged: Vec<(i64, String, String, i64)> = connection
        .prepare("SELECT seq, table_name, row_key, changed_ms FROM notia_row_changes WHERE pending = 1 ORDER BY seq LIMIT ?1")?
        .query_map([limit as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut changes = Vec::new();
    for (seq, change) in changes_of(connection, logged)? {
        match change {
            Some(change) => changes.push(change),
            None => {
                connection.execute("UPDATE notia_row_changes SET pending = 0 WHERE seq = ?1", [seq])?;
            }
        }
    }
    Ok(changes)
}

/// The host took `changes`: they stop being pending unless this device
/// changed the row again meanwhile.
pub(crate) fn mark_sent(connection: &Connection, changes: &[RowChange]) -> rusqlite::Result<()> {
    let transaction = connection.unchecked_transaction()?;
    for change in changes {
        let Some(key) = change.key.iter().map(from_json).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let sql = format!(
            "UPDATE notia_row_changes SET pending = 0 WHERE table_name = ?1 AND changed_ms = ?2 AND row_key = {}",
            key_expression(key.len(), 3)
        );
        let mut values = vec![SqlValue::Text(change.table.clone()), SqlValue::Integer(change.changed_ms)];
        values.extend(key);
        transaction.execute(&sql, params_from_iter(values.iter()))?;
    }
    transaction.commit()
}

/// Whether this device has changes the host has not taken.
#[cfg(test)]
pub(crate) fn has_pending(connection: &Connection) -> rusqlite::Result<bool> {
    connection
        .query_row("SELECT EXISTS(SELECT 1 FROM notia_row_changes WHERE pending = 1)", [], |row| row.get(0))
        .or_else(|error| if missing_table(&error) { Ok(false) } else { Err(error) })
}

/// Marks every logged row as taken: a fresh snapshot of the host has no
/// changes of this device.
pub(crate) fn clear_pending(connection: &Connection) -> rusqlite::Result<()> {
    match connection.execute("UPDATE notia_row_changes SET pending = 0 WHERE pending = 1", []) {
        Err(error) if missing_table(&error) => Ok(()),
        other => other.map(|_| ()),
    }
}

// ---------- Both ----------

/// Applies the changes of the other side, newest change wins (see the
/// module). Each applied row is logged with its own time, not pending.
pub(crate) fn apply_changes(connection: &Connection, changes: &[RowChange]) -> rusqlite::Result<ApplyReport> {
    let foreign_keys: bool = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    connection.execute_batch("PRAGMA foreign_keys = OFF")?;
    let result = apply_all(connection, changes);
    let restored = connection.execute_batch(if foreign_keys { "PRAGMA foreign_keys = ON" } else { "PRAGMA foreign_keys = OFF" });
    let report = result?;
    restored?;
    Ok(report)
}

fn apply_all(connection: &Connection, changes: &[RowChange]) -> rusqlite::Result<ApplyReport> {
    let transaction = connection.unchecked_transaction()?;
    let mut tables = Tables::new(connection);
    let mut report = ApplyReport::default();
    for change in changes {
        let Some(info) = tables.get(&change.table)?.cloned() else {
            report.failed += 1;
            continue;
        };
        let key = change.key.iter().map(from_json).collect::<Option<Vec<_>>>().filter(|key| key.len() == info.key.len());
        let Some(key) = key else {
            report.failed += 1;
            continue;
        };
        let mut lookup = vec![SqlValue::Text(change.table.clone())];
        lookup.extend(key.iter().cloned());
        let known: Option<i64> = connection
            .query_row(
                &format!(
                    "SELECT changed_ms FROM notia_row_changes WHERE table_name = ?1 AND row_key = {}",
                    key_expression(key.len(), 2)
                ),
                params_from_iter(lookup.iter()),
                |row| row.get(0),
            )
            .optional()?;
        if known.is_some_and(|known| known >= change.changed_ms) {
            report.kept += 1;
            continue;
        }
        let written = match &change.row {
            Some(row) => upsert(connection, &change.table, &info, row),
            None => connection
                .execute(&format!("DELETE FROM {} WHERE {}", quoted(&change.table), key_filter(&info)), params_from_iter(key.iter()))
                .map(|_| true),
        };
        match written {
            Ok(true) => {}
            Ok(false) => {
                report.failed += 1;
                continue;
            }
            // A constraint of this side (another row with the same unique
            // value): the row stays as it is here.
            Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::ConstraintViolation => {
                log::error!("[notia:copy] a row of {} did not fit this database", change.table);
                report.failed += 1;
                continue;
            }
            Err(error) => return Err(error),
        }
        let mut logged = vec![SqlValue::Text(change.table.clone()), SqlValue::Integer(change.changed_ms)];
        logged.extend(key);
        connection.execute(
            &format!(
                "INSERT OR REPLACE INTO notia_row_changes (table_name, row_key, changed_ms, pending) VALUES (?1, {}, ?2, 0)",
                key_expression(info.key.len(), 3)
            ),
            params_from_iter(logged.iter()),
        )?;
        report.applied += 1;
    }
    transaction.commit()?;
    Ok(report)
}

/// Inserts or replaces the row with the columns both sides have. `false`
/// when the row lacks its key or a value cannot be read.
fn upsert(connection: &Connection, table: &str, info: &TableInfo, row: &Map<String, Value>) -> rusqlite::Result<bool> {
    let mut columns = Vec::new();
    let mut values = Vec::new();
    for column in &info.columns {
        if let Some(value) = row.get(column) {
            let Some(value) = from_json(value) else {
                return Ok(false);
            };
            columns.push(column.as_str());
            values.push(value);
        }
    }
    if info.key.iter().any(|key| !columns.contains(&key.as_str())) {
        return Ok(false);
    }
    let names: Vec<String> = columns.iter().map(|column| quoted(column)).collect();
    let placeholders: Vec<String> = (1..=columns.len()).map(|index| format!("?{index}")).collect();
    let updates: Vec<String> = columns
        .iter()
        .filter(|column| !info.key.iter().any(|key| key == *column))
        .map(|column| format!("{0} = excluded.{0}", quoted(column)))
        .collect();
    let conflict: Vec<String> = info.key.iter().map(|column| quoted(column)).collect();
    let action = if updates.is_empty() { "NOTHING".to_string() } else { format!("UPDATE SET {}", updates.join(", ")) };
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO {action}",
        quoted(table),
        names.join(", "),
        placeholders.join(", "),
        conflict.join(", "),
    );
    connection.execute(&sql, params_from_iter(values.iter()))?;
    Ok(true)
}

/// The rows of `table` by key, for tests.
#[cfg(test)]
fn dump(connection: &Connection, table: &str) -> std::collections::BTreeMap<String, Map<String, Value>> {
    let info = table_info(connection, table).expect("info").expect("tracked");
    let columns: Vec<String> = info.columns.iter().map(|column| quoted(column)).collect();
    let mut statement = connection.prepare(&format!("SELECT {} FROM {}", columns.join(", "), quoted(table))).expect("select");
    let rows = statement
        .query_map([], |row| {
            let mut values = Map::new();
            for (index, column) in info.columns.iter().enumerate() {
                values.insert(column.clone(), to_json(row.get_ref(index)?));
            }
            Ok(values)
        })
        .expect("rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows.into_iter().map(|row| (Value::Array(info.key.iter().map(|key| row[key].clone()).collect()).to_string(), row)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn library() -> Connection {
        let connection = Connection::open_in_memory().expect("SQLite");
        crate::database::migrate(&connection).expect("migration");
        connection
    }

    /// A copy as a client gets it: the host's database with nothing pending.
    fn copy_of(host: &Connection) -> (Connection, PathBuf) {
        let path = std::env::temp_dir().join(format!("notia-row-sync-{}.db", uuid::Uuid::new_v4()));
        host.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()]).expect("snapshot");
        let copy = Connection::open(&path).expect("copy");
        clear_pending(&copy).expect("clear");
        crate::database::migrate(&copy).expect("migrate copy");
        (copy, path)
    }

    fn remove(copy: Connection, path: PathBuf) {
        drop(copy);
        let _ = std::fs::remove_file(path);
    }

    /// Sets when a logged row of `table` changed.
    fn stamp(connection: &Connection, table: &str, row_key: &str, ms: i64) {
        let changed = connection
            .execute(
                "UPDATE notia_row_changes SET changed_ms = ?3 WHERE table_name = ?1 AND row_key = ?2",
                rusqlite::params![table, row_key, ms],
            )
            .expect("stamp");
        assert_eq!(changed, 1, "{table} {row_key}");
    }

    fn day(date: &str) -> String {
        format!("[\"user-owner\",\"{date}\"]")
    }

    fn water(connection: &Connection, date: &str) -> Option<i64> {
        connection
            .query_row("SELECT ml FROM health_water WHERE owner_user_id='user-owner' AND date=?1", [date], |row| row.get(0))
            .optional()
            .expect("water")
    }

    /// Takes what the copy changed to the host, then what the host changed
    /// to the copy, as `host_mirror` does.
    fn reconcile(host: &Connection, copy: &Connection, since: i64) -> (ApplyReport, ApplyReport, i64) {
        let pending = pending_changes(copy, 500).expect("pending");
        let pushed = apply_changes(host, &pending).expect("push");
        mark_sent(copy, &pending).expect("sent");
        let batch = changes_since(host, since, 500).expect("changes");
        assert!(!batch.more);
        let pulled = apply_changes(copy, &batch.changes).expect("pull");
        (pushed, pulled, batch.last_seq)
    }

    #[test]
    fn every_library_table_is_tracked_except_the_device_ones() {
        let connection = library();
        let untracked: Vec<String> = table_names(&connection)
            .expect("tables")
            .into_iter()
            .filter(|table| table_info(&connection, table).expect("info").is_none())
            .collect();
        assert!(untracked.is_empty(), "tables without a usable key: {untracked:?}");
        let triggers: i64 = connection
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'notia_track_v1_%'", [], |row| row.get(0))
            .expect("triggers");
        assert_eq!(triggers as usize, table_names(&connection).expect("tables").len() * 3);
        // Opening again leaves the triggers as they are.
        ensure_tracking(&connection).expect("again");
        assert!(database_id(&connection).expect("id").is_some_and(|id| id.len() == 32));
    }

    #[test]
    fn the_log_follows_inserts_updates_key_changes_and_deletes() {
        let connection = library();
        clear_pending(&connection).expect("clear");
        connection
            .execute_batch(
                "INSERT INTO agenda_notes (id, owner_user_id, text, created_at, updated_at) VALUES ('n1', 'user-owner', 'hola', 'x', 'x');
                 UPDATE agenda_notes SET text = 'chau' WHERE id = 'n1';
                 UPDATE agenda_notes SET id = 'n2' WHERE id = 'n1';",
            )
            .expect("writes");
        let summary = |changes: Vec<RowChange>| -> Vec<(String, Option<String>)> {
            changes
                .into_iter()
                .map(|change| {
                    let key = change.key[0].as_str().unwrap_or_default().to_string();
                    (key, change.row.map(|row| row["text"].as_str().unwrap_or_default().to_string()))
                })
                .collect()
        };
        assert_eq!(
            summary(pending_changes(&connection, 10).expect("pending")),
            vec![("n1".to_string(), None), ("n2".to_string(), Some("chau".to_string()))]
        );
        connection.execute("DELETE FROM agenda_notes WHERE id = 'n2'", []).expect("delete");
        connection.execute("INSERT INTO ai_action_meta (key, value) VALUES ('lease', 'x')", []).expect("lease");
        assert_eq!(
            summary(pending_changes(&connection, 10).expect("pending")),
            vec![("n1".to_string(), None), ("n2".to_string(), None)],
            "deletions travel; the device tables never do"
        );
    }

    #[test]
    fn host_and_copy_keep_the_changes_of_both_and_the_newest_wins() {
        let host = library();
        host.execute_batch(
            "INSERT INTO health_water (owner_user_id, date, ml, updated_at) VALUES
                 ('user-owner', '2026-10-01', 500, 1), ('user-owner', '2026-10-02', 750, 1), ('user-owner', '2026-10-03', 250, 1);
             UPDATE notia_row_changes SET changed_ms = 500;",
        )
        .expect("rows");
        let (copy, path) = copy_of(&host);
        let since = last_seq(&host).expect("seq");
        assert!(!has_pending(&copy).expect("pending"), "a fresh copy has nothing to send");

        // Offline: each side adds a day; both change the 2nd (the copy
        // later) and the 3rd (the host later); the copy deletes the 1st.
        copy.execute_batch(
            "INSERT INTO health_water (owner_user_id, date, ml, updated_at) VALUES ('user-owner', '2026-10-04', 1000, 2);
             UPDATE health_water SET ml = 900 WHERE date = '2026-10-02';
             UPDATE health_water SET ml = 300 WHERE date = '2026-10-03';
             DELETE FROM health_water WHERE date = '2026-10-01';",
        )
        .expect("copy");
        host.execute_batch(
            "INSERT INTO health_water (owner_user_id, date, ml, updated_at) VALUES ('user-owner', '2026-10-05', 1250, 2);
             UPDATE health_water SET ml = 800 WHERE date = '2026-10-02';
             UPDATE health_water SET ml = 350 WHERE date = '2026-10-03';",
        )
        .expect("host");
        stamp(&copy, "health_water", &day("2026-10-02"), 2_000);
        stamp(&host, "health_water", &day("2026-10-02"), 1_000);
        stamp(&copy, "health_water", &day("2026-10-03"), 1_000);
        stamp(&host, "health_water", &day("2026-10-03"), 2_000);

        let (pushed, pulled, since) = reconcile(&host, &copy, since);
        assert_eq!((pushed.applied, pushed.kept, pushed.failed), (3, 1, 0), "{pushed:?}");
        assert_eq!(pulled.failed, 0, "{pulled:?}");
        for connection in [&host, &copy] {
            assert_eq!(water(connection, "2026-10-01"), None, "the copy's deletion");
            assert_eq!(water(connection, "2026-10-02"), Some(900), "the copy changed it later");
            assert_eq!(water(connection, "2026-10-03"), Some(350), "the host changed it later");
            assert_eq!(water(connection, "2026-10-04"), Some(1000), "added by the copy");
            assert_eq!(water(connection, "2026-10-05"), Some(1250), "added by the host");
        }
        assert_eq!(dump(&host, "health_water"), dump(&copy, "health_water"));
        assert!(!has_pending(&copy).expect("pending"));

        // Nothing new: nothing travels.
        let (pushed, pulled, _) = reconcile(&host, &copy, since);
        assert_eq!((pushed.applied, pulled.applied), (0, 0));
        remove(copy, path);
    }

    #[test]
    fn values_keep_their_type_and_rows_with_other_columns_still_travel() {
        let host = library();
        host.execute_batch(
            "INSERT INTO health_weights (owner_user_id, date, kg, updated_at) VALUES ('user-owner', '2026-10-01', 80.0, 1);
             CREATE TABLE extra_blobs (id TEXT PRIMARY KEY, data BLOB, only_host TEXT);",
        )
        .expect("rows");
        crate::database::migrate(&host).expect("triggers for the new table");
        let (copy, path) = copy_of(&host);
        copy.execute_batch("ALTER TABLE extra_blobs ADD COLUMN only_copy TEXT;").expect("column");
        let since = last_seq(&host).expect("seq");
        copy.execute("INSERT INTO extra_blobs (id, data, only_host, only_copy) VALUES ('b', ?1, 'h', 'c')", [vec![0_u8, 1, 2, 255]])
            .expect("blob");
        copy.execute("UPDATE health_weights SET kg = 79.5 WHERE date = '2026-10-01'", []).expect("weight");
        let (pushed, _, _) = reconcile(&host, &copy, since);
        assert_eq!(pushed.applied, 2, "{pushed:?}");
        let (data, only_host): (Vec<u8>, String) = host
            .query_row("SELECT data, only_host FROM extra_blobs WHERE id = 'b'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("blob on the host");
        assert_eq!((data, only_host.as_str()), (vec![0, 1, 2, 255], "h"));
        let (kind, kg): (String, f64) = host
            .query_row("SELECT typeof(kg), kg FROM health_weights WHERE date = '2026-10-01'", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("weight on the host");
        assert_eq!((kind.as_str(), kg), ("real", 79.5));
        remove(copy, path);
    }

    #[test]
    fn a_row_that_breaks_a_unique_value_of_the_other_side_is_left_out() {
        let host = library();
        let (copy, path) = copy_of(&host);
        let since = last_seq(&host).expect("seq");
        // One budget per category and currency.
        let budget =
            "INSERT INTO finance_budgets (id, category_id, currency, amount, created_at, updated_at) VALUES (?1, 'category-other', 'ARS', '100.00', 'x', 'x')";
        copy.execute(budget, ["copia"]).expect("copy budget");
        host.execute(budget, ["host"]).expect("host budget");
        let (pushed, pulled, _) = reconcile(&host, &copy, since);
        assert_eq!((pushed.applied, pushed.failed), (0, 1), "{pushed:?}");
        assert_eq!(pulled.failed, 1, "{pulled:?}");
        let ids = |connection: &Connection| -> Vec<String> {
            connection
                .prepare("SELECT id FROM finance_budgets WHERE category_id = 'category-other'")
                .expect("query")
                .query_map([], |row| row.get(0))
                .expect("rows")
                .collect::<Result<_, _>>()
                .expect("ids")
        };
        assert_eq!((ids(&host), ids(&copy)), (vec!["host".to_string()], vec!["copia".to_string()]));
        remove(copy, path);
    }

    #[test]
    fn the_host_hands_its_changes_in_order_and_in_batches() {
        let host = library();
        for id in ["a", "b", "c"] {
            host.execute(
                "INSERT INTO agenda_notes (id, owner_user_id, text, created_at, updated_at) VALUES (?1, 'user-owner', 'x', 'x', 'x')",
                [id],
            )
            .expect("note");
        }
        let last = last_seq(&host).expect("seq");
        let first = changes_since(&host, last - 3, 2).expect("first");
        assert!(first.more);
        let second = changes_since(&host, first.last_seq, 2).expect("second");
        assert!(!second.more);
        let ids: Vec<&str> = first.changes.iter().chain(&second.changes).map(|change| change.key[0].as_str().unwrap_or_default()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(second.last_seq, last);
        assert!(changes_since(&host, last, 10).expect("none").changes.is_empty());
    }
}

