//! AI actions of a library (the rules are in `backend_core::ai_actions`).
//!
//! Actions and runs live in the library's SQLite database, so every device
//! of the library sees the same dashboard. The clock runs on one instance
//! only: the one where the library's Telegram bot runs and that holds the
//! lease stored with the actions (the headless server takes it from the
//! desktop app, the desktop app from Android). Each due occurrence becomes
//! a run, and the run goes through the Telegram worker as a turn of the
//! Owner's chat (`telegram_worker::enqueue_action`), which reports back
//! with `mark_running` and `finish_run`.
//!
//! Every change emits `ACTION_CHANGED_EVENT` or `RUN_UPDATED_EVENT` with
//! the library id, and the dashboard reloads.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::ai_actions::dashboard::{run_history, RunRow};
use crate::backend::ai_actions::labels::{form_summary, moment_label, schedule_label};
use crate::backend::ai_actions::prompts::{action_request, ScheduledActionPrompt};
use crate::backend::ai_actions::scheduler::{
    claim_lease, interrupted_runs, lease_needs_renewal, plan_action, InstanceKind, Planned, SchedulerLease, INTERRUPTED,
};
use crate::backend::ai_actions::schedule::local_date;
use crate::backend::ai_actions::tools;
use crate::backend::ai_actions::{
    self as core, build_dashboard, time_zone, upcoming, validate_input, AiAction, AiActionInput, AiActionKind, AiActionRun,
    AiActionsDashboard, AiSchedule, DashboardFilter, FieldError, InputOrigin, RunStatus, RunTrigger, ValidAction, PREVIEW_OCCURRENCES,
};
use crate::host::{AppHandle, Emitter, Manager};
use crate::library_users::LibraryDatabaseContext;
use crate::telegram_worker::{enqueue_action, ActionEnqueue, ActionJob};

pub(crate) const ACTION_CHANGED_EVENT: &str = "notia://ai-action-changed";
pub(crate) const RUN_UPDATED_EVENT: &str = "notia://ai-run-updated";

const TICK: Duration = Duration::from_secs(15);
/// The first tick waits for the app to open its library and its bot.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(20);
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Runs the dashboard reads: today's, and yesterday's for the day change.
const RECENT_RUNS_MS: i64 = 2 * 24 * 60 * 60 * 1000;
/// Runs are kept this long for the history.
const RUN_RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
const LEASE_KEY: &str = "scheduler.lease";
const LAST_CHECK_KEY: &str = "scheduler.lastCheckMs";
const HOURLY_REVIEW_SEEDED_KEY: &str = "builtin.hourly-review.seeded";
const STATE_DIRECTORY: &str = "ai-actions";
const TEST_NAME: &str = "Prueba";

// ---------------------------------------------------------------------------
// Errors and context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiActionsErrorCode {
    Validation,
    NotFound,
    Forbidden,
    Unavailable,
    Storage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionsError {
    pub code: AiActionsErrorCode,
    pub message: String,
    /// The form's errors by field, for `Validation`.
    pub fields: Vec<FieldError>,
}

impl AiActionsError {
    fn new(code: AiActionsErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), fields: Vec::new() }
    }

    fn validation(fields: Vec<FieldError>) -> Self {
        Self { code: AiActionsErrorCode::Validation, message: "Revisá los campos marcados.".into(), fields }
    }

    fn storage(message: impl Into<String>) -> Self {
        Self::new(AiActionsErrorCode::Storage, message)
    }

    fn not_found() -> Self {
        Self::new(AiActionsErrorCode::NotFound, "La acción ya no existe.")
    }
}

impl From<rusqlite::Error> for AiActionsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::storage(format!("No se pudo acceder a las acciones de IA: {error}"))
    }
}

pub type AiActionsResult<T> = Result<T, AiActionsError>;

/// Library of a command. The actions are the Owner's only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionsContext {
    pub library_id: String,
    pub library_path: String,
    pub android_directory_uri: Option<String>,
    pub actor_library_user_id: String,
}

impl AiActionsContext {
    fn database(&self) -> AiActionsResult<LibraryDatabaseContext> {
        if self.actor_library_user_id.trim() != crate::backend::OWNER_LIBRARY_USER_ID {
            return Err(AiActionsError::new(AiActionsErrorCode::Forbidden, "Las acciones de IA son solo del Owner de la biblioteca."));
        }
        Ok(LibraryDatabaseContext { library_path: self.library_path.clone(), android_directory_uri: self.android_directory_uri.clone() })
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as i64).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

fn open(app: &AppHandle, database: &LibraryDatabaseContext) -> AiActionsResult<Connection> {
    let connection = crate::database::open_user_data_connection(app, &database.library_path, database.android_directory_uri.as_deref())
        .map_err(AiActionsError::storage)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    Ok(connection)
}

/// Runs `work` in a transaction and saves it (on Android, back through SAF).
fn write<T>(app: &AppHandle, database: &LibraryDatabaseContext, work: impl FnOnce(&Connection) -> AiActionsResult<T>) -> AiActionsResult<T> {
    let mut connection = open(app, database)?;
    let transaction = connection.transaction()?;
    let value = work(&transaction)?;
    transaction.commit()?;
    drop(connection);
    crate::database::sync_user_data_connection(app, database.android_directory_uri.as_deref()).map_err(AiActionsError::storage)?;
    Ok(value)
}

fn read<T>(app: &AppHandle, database: &LibraryDatabaseContext, work: impl FnOnce(&Connection) -> AiActionsResult<T>) -> AiActionsResult<T> {
    work(&open(app, database)?)
}

fn corrupt(what: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(0, what.to_string(), rusqlite::types::Type::Text)
}

fn action_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiAction> {
    let kind: String = row.get(3)?;
    let schedule: String = row.get(4)?;
    Ok(AiAction {
        id: row.get(0)?,
        name: row.get(1)?,
        prompt: row.get(2)?,
        kind: AiActionKind::from_id(&kind).ok_or_else(|| corrupt("kind"))?,
        schedule: serde_json::from_str::<AiSchedule>(&schedule).map_err(|_| corrupt("schedule_json"))?,
        time_zone: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        active_since_ms: row.get(7)?,
        builtin: row.get(8)?,
        created_at_ms: row.get(9)?,
        updated_at_ms: row.get(10)?,
    })
}

const ACTION_COLUMNS: &str = "id, name, prompt, kind, schedule_json, time_zone, enabled, active_since, builtin, created_at, updated_at";

fn load_actions(connection: &Connection) -> AiActionsResult<Vec<AiAction>> {
    let mut statement = connection.prepare(&format!("SELECT {ACTION_COLUMNS} FROM ai_actions ORDER BY created_at, id"))?;
    let actions = statement.query_map([], action_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(actions)
}

fn load_action(connection: &Connection, id: &str) -> AiActionsResult<AiAction> {
    connection
        .query_row(&format!("SELECT {ACTION_COLUMNS} FROM ai_actions WHERE id=?1"), [id], action_from_row)
        .optional()?
        .ok_or_else(AiActionsError::not_found)
}

fn save_action(connection: &Connection, action: &AiAction) -> AiActionsResult<()> {
    let schedule = serde_json::to_string(&action.schedule).map_err(|_| AiActionsError::storage("No se pudo guardar el horario."))?;
    connection.execute(
        "INSERT INTO ai_actions (id, name, prompt, kind, schedule_json, time_zone, enabled, active_since, builtin, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, prompt=excluded.prompt, kind=excluded.kind,
             schedule_json=excluded.schedule_json, time_zone=excluded.time_zone, enabled=excluded.enabled,
             active_since=excluded.active_since, updated_at=excluded.updated_at",
        params![
            action.id,
            action.name,
            action.prompt,
            action.kind.id(),
            schedule,
            action.time_zone,
            i64::from(action.enabled),
            action.active_since_ms,
            action.builtin,
            action.created_at_ms,
            action.updated_at_ms
        ],
    )?;
    Ok(())
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiActionRun> {
    let kind: String = row.get(3)?;
    let status: String = row.get(8)?;
    let trigger: String = row.get(11)?;
    Ok(AiActionRun {
        id: row.get(0)?,
        action_id: row.get(1)?,
        action_name: row.get(2)?,
        kind: AiActionKind::from_id(&kind).ok_or_else(|| corrupt("kind"))?,
        scheduled_for_ms: row.get(4)?,
        created_at_ms: row.get(5)?,
        started_at_ms: row.get(6)?,
        finished_at_ms: row.get(7)?,
        status: RunStatus::from_id(&status).ok_or_else(|| corrupt("status"))?,
        error: row.get(9)?,
        output_summary: row.get(10)?,
        trigger: RunTrigger::from_id(&trigger).ok_or_else(|| corrupt("trigger"))?,
        retry_of: row.get(12)?,
        runner: row.get(13)?,
    })
}

const RUN_COLUMNS: &str =
    "id, action_id, action_name, kind, scheduled_for, created_at, started_at, finished_at, status, error, output_summary, trigger, retry_of, runner";

fn query_runs(connection: &Connection, filter: &str, value: impl rusqlite::ToSql) -> AiActionsResult<Vec<AiActionRun>> {
    let mut statement = connection.prepare(&format!("SELECT {RUN_COLUMNS} FROM ai_action_runs WHERE {filter} ORDER BY created_at, id"))?;
    let runs = statement.query_map([value], run_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(runs)
}

/// The recent runs, plus every run of the one-time actions (whose single
/// occurrence may be older).
fn planning_runs(connection: &Connection, now: i64) -> AiActionsResult<Vec<AiActionRun>> {
    query_runs(
        connection,
        "scheduled_for >= ?1 OR status IN ('pending', 'running') OR action_id IN (SELECT id FROM ai_actions WHERE kind != 'recurring')",
        now - RECENT_RUNS_MS,
    )
}

fn load_run(connection: &Connection, id: &str) -> AiActionsResult<AiActionRun> {
    query_runs(connection, "id = ?1", id)?
        .into_iter()
        .next()
        .ok_or_else(|| AiActionsError::new(AiActionsErrorCode::NotFound, "La ejecución ya no existe."))
}

/// Inserts a run; false when the occurrence already had its scheduled run.
fn insert_run(connection: &Connection, run: &AiActionRun) -> AiActionsResult<bool> {
    let inserted = connection.execute(
        &format!("INSERT OR IGNORE INTO ai_action_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
        params![
            run.id,
            run.action_id,
            run.action_name,
            run.kind.id(),
            run.scheduled_for_ms,
            run.created_at_ms,
            run.started_at_ms,
            run.finished_at_ms,
            run.status.id(),
            run.error,
            run.output_summary,
            run.trigger.id(),
            run.retry_of,
            run.runner
        ],
    )?;
    Ok(inserted == 1)
}

fn update_run(connection: &Connection, run: &AiActionRun) -> AiActionsResult<()> {
    connection.execute(
        "UPDATE ai_action_runs SET started_at=?2, finished_at=?3, status=?4, error=?5, output_summary=?6 WHERE id=?1",
        params![run.id, run.started_at_ms, run.finished_at_ms, run.status.id(), run.error, run.output_summary],
    )?;
    Ok(())
}

fn meta(connection: &Connection, key: &str) -> AiActionsResult<Option<String>> {
    Ok(connection.query_row("SELECT value FROM ai_action_meta WHERE key=?1", [key], |row| row.get(0)).optional()?)
}

fn set_meta(connection: &Connection, key: &str, value: &str) -> AiActionsResult<()> {
    connection.execute(
        "INSERT INTO ai_action_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Library settings
// ---------------------------------------------------------------------------

/// The library's zone: its place's (Settings → Clima), else the system's.
fn library_zone(app: &AppHandle, library_id: &str) -> String {
    let config = crate::library_config::read_library_config(app, library_id).ok().flatten();
    let place = crate::backend::weather::configured_location(config.as_ref()).0;
    core::library_time_zone(place.timezone.as_deref())
}

/// Sets up the hourly review once per library. It starts on when the
/// library had the autonomous agent on, the switch that ran it before.
fn ensure_defaults(app: &AppHandle, library_id: &str, connection: &Connection, now: i64) -> AiActionsResult<bool> {
    if meta(connection, HOURLY_REVIEW_SEEDED_KEY)?.is_some() {
        return Ok(false);
    }
    let enabled = crate::library_config::read_library_config(app, library_id)
        .ok()
        .flatten()
        .is_none_or(|config| crate::backend::library_config::autonomous_agent_enabled(&config));
    let review = core::hourly_review_action(Uuid::new_v4().to_string(), library_zone(app, library_id), enabled, now);
    save_action(connection, &review)?;
    set_meta(connection, HOURLY_REVIEW_SEEDED_KEY, "1")?;
    Ok(true)
}

fn emit(app: &AppHandle, event: &str, library_id: &str) {
    let _ = app.emit(event, library_id);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Runs the database work of a command off the async runtime.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> AiActionsResult<T> + Send + 'static) -> AiActionsResult<T> {
    crate::host::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| AiActionsError::storage("No se pudo acceder a las acciones de IA."))?
}

pub async fn ai_actions_dashboard(
    app: AppHandle,
    context: AiActionsContext,
    filter: Option<DashboardFilter>,
    query: Option<String>,
) -> AiActionsResult<AiActionsDashboard> {
    blocking(move || {
        let database = context.database()?;
        let now = now_ms();
        let seeded = write(&app, &database, |connection| ensure_defaults(&app, &context.library_id, connection, now))?;
        if seeded {
            emit(&app, ACTION_CHANGED_EVENT, &context.library_id);
        }
        let zone = time_zone(&library_zone(&app, &context.library_id));
        read(&app, &database, |connection| {
            let actions = load_actions(connection)?;
            let runs = query_runs(connection, "scheduled_for >= ?1", now - RECENT_RUNS_MS)?;
            Ok(build_dashboard(&actions, &runs, now, &zone, filter.unwrap_or_default(), query.as_deref().unwrap_or_default()))
        })
    })
    .await
}

/// What the form shows while it is filled: its errors, the summary and the
/// next occurrences.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPreview {
    pub errors: Vec<FieldError>,
    pub summary: String,
    pub when: Option<String>,
    pub upcoming: Vec<String>,
}

/// The form of an action, to edit it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionForm {
    pub input: AiActionInput,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedAction {
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedRun {
    pub run_id: String,
}

fn origin_of(action: &AiAction) -> InputOrigin<'_> {
    match &action.schedule {
        AiSchedule::Once { at_ms } => InputOrigin { stored_at_ms: Some(*at_ms), stored_anchor_date: None },
        AiSchedule::Recurring { rule } => InputOrigin { stored_at_ms: None, stored_anchor_date: Some(rule.anchor_date.as_str()) },
    }
}

fn preview(input: &AiActionInput, zone_name: &str, stored: Option<&AiAction>, now: i64) -> ActionPreview {
    let zone = time_zone(zone_name);
    let today = local_date(&zone, now);
    let mut weekdays = input.weekdays.clone();
    weekdays.sort_unstable();
    weekdays.dedup();
    let summary = input.kind.map(|kind| form_summary(kind, &weekdays)).unwrap_or_default();
    match validate_input(input, &zone, now, stored.map(origin_of).unwrap_or_default()) {
        Ok(valid) => ActionPreview {
            errors: Vec::new(),
            summary,
            when: Some(schedule_label(&valid.schedule, &zone, today)),
            upcoming: upcoming(&valid.schedule, &zone, now, PREVIEW_OCCURRENCES).into_iter().map(|ms| moment_label(&zone, ms, today)).collect(),
        },
        Err(errors) => ActionPreview { errors, summary, when: None, upcoming: Vec::new() },
    }
}

pub async fn ai_action_preview(
    app: AppHandle,
    context: AiActionsContext,
    action_id: Option<String>,
    input: AiActionInput,
) -> AiActionsResult<ActionPreview> {
    blocking(move || {
        let database = context.database()?;
        let stored = match action_id.as_deref() {
            Some(id) => Some(read(&app, &database, |connection| load_action(connection, id))?),
            None => None,
        };
        let zone = stored.as_ref().map(|action| action.time_zone.clone()).unwrap_or_else(|| library_zone(&app, &context.library_id));
        Ok(preview(&input, &zone, stored.as_ref(), now_ms()))
    })
    .await
}

pub async fn ai_action_get(app: AppHandle, context: AiActionsContext, action_id: String) -> AiActionsResult<ActionForm> {
    blocking(move || {
        let database = context.database()?;
        let action = read(&app, &database, |connection| load_action(connection, &action_id))?;
        Ok(ActionForm { input: core::input_from_action(&action), builtin: action.builtin.is_some() })
    })
    .await
}

// Shared by the panel's commands and the agent's tools, so both change the
// actions the same way.

fn new_action(valid: ValidAction, time_zone: String, enabled: bool, now: i64) -> AiAction {
    AiAction {
        id: Uuid::new_v4().to_string(),
        name: valid.name,
        prompt: valid.prompt,
        kind: valid.kind,
        schedule: valid.schedule,
        time_zone,
        enabled,
        active_since_ms: now,
        created_at_ms: now,
        updated_at_ms: now,
        builtin: None,
    }
}

/// The action after an edit. A new schedule, and turning it on again,
/// start now: what it would have run before is neither run nor lost.
fn edited(action: &AiAction, valid: ValidAction, enabled: Option<bool>, now: i64) -> AiAction {
    let mut next = action.clone();
    if valid.schedule != next.schedule || valid.kind != next.kind {
        next.active_since_ms = now;
    }
    next.name = valid.name;
    next.prompt = valid.prompt;
    next.kind = valid.kind;
    next.schedule = valid.schedule;
    if let Some(enabled) = enabled {
        if enabled && !next.enabled {
            next.active_since_ms = now;
        }
        next.enabled = enabled;
    }
    next.updated_at_ms = now;
    next
}

fn create_action(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, input: &AiActionInput, enabled: bool) -> AiActionsResult<AiAction> {
    let now = now_ms();
    let zone_name = library_zone(app, library_id);
    let valid = validate_input(input, &time_zone(&zone_name), now, InputOrigin::default()).map_err(AiActionsError::validation)?;
    let action = new_action(valid, zone_name, enabled, now);
    write(app, database, |connection| save_action(connection, &action))?;
    emit(app, ACTION_CHANGED_EVENT, library_id);
    Ok(action)
}

fn update_action(
    app: &AppHandle,
    library_id: &str,
    database: &LibraryDatabaseContext,
    action_id: &str,
    input: &AiActionInput,
    enabled: Option<bool>,
) -> AiActionsResult<AiAction> {
    let now = now_ms();
    let action = write(app, database, |connection| {
        let action = load_action(connection, action_id)?;
        let valid = validate_input(input, &time_zone(&action.time_zone), now, origin_of(&action)).map_err(AiActionsError::validation)?;
        let next = edited(&action, valid, enabled, now);
        save_action(connection, &next)?;
        Ok(next)
    })?;
    emit(app, ACTION_CHANGED_EVENT, library_id);
    Ok(action)
}

fn set_action_enabled(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, action_id: &str, enabled: bool) -> AiActionsResult<AiAction> {
    let now = now_ms();
    let action = write(app, database, |connection| {
        let mut action = load_action(connection, action_id)?;
        if action.enabled == enabled {
            return Ok(action);
        }
        // Turned on again, the occurrences lost meanwhile are not recovered.
        if enabled {
            action.active_since_ms = now;
        }
        action.enabled = enabled;
        action.updated_at_ms = now;
        save_action(connection, &action)?;
        Ok(action)
    })?;
    emit(app, ACTION_CHANGED_EVENT, library_id);
    Ok(action)
}

/// Deletes the action; its runs stay in the history.
fn delete_action(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, action_id: &str) -> AiActionsResult<AiAction> {
    let action = write(app, database, |connection| {
        let action = load_action(connection, action_id)?;
        connection.execute("DELETE FROM ai_actions WHERE id=?1", [action_id])?;
        Ok(action)
    })?;
    emit(app, ACTION_CHANGED_EVENT, library_id);
    Ok(action)
}

fn telegram_unavailable() -> AiActionsError {
    AiActionsError::new(
        AiActionsErrorCode::Unavailable,
        "Telegram no está activo en este equipo o el Owner no vinculó su chat: la IA responde siempre por Telegram.",
    )
}

fn ensure_telegram(app: &AppHandle, library_id: &str) -> AiActionsResult<()> {
    if crate::telegram_worker::bot_runs_for(app, library_id) {
        Ok(())
    } else {
        Err(telegram_unavailable())
    }
}

fn pending_run(action: &AiAction, scheduled_for_ms: i64, trigger: RunTrigger, retry_of: Option<String>, app: &AppHandle) -> AiActionRun {
    AiActionRun {
        id: Uuid::new_v4().to_string(),
        action_id: Some(action.id.clone()),
        action_name: action.name.clone(),
        kind: action.kind,
        scheduled_for_ms,
        created_at_ms: now_ms(),
        started_at_ms: None,
        finished_at_ms: None,
        status: RunStatus::Pending,
        error: None,
        output_summary: None,
        trigger,
        retry_of,
        runner: Some(instance_id(app)),
    }
}

/// The failed run and its action, when it can be retried.
fn retryable(connection: &Connection, run_id: &str) -> AiActionsResult<(AiActionRun, AiAction)> {
    let failed = load_run(connection, run_id)?;
    if failed.status != RunStatus::Failed {
        return Err(AiActionsError::new(AiActionsErrorCode::Validation, "Solo se puede reintentar una ejecución que falló."));
    }
    let action = load_action(connection, failed.action_id.as_deref().unwrap_or_default())?;
    Ok((failed, action))
}

/// Retries a failed run now, with the action's current prompt.
fn retry_run(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, run_id: &str) -> AiActionsResult<AiActionRun> {
    ensure_telegram(app, library_id)?;
    let (run, action) = write(app, database, |connection| {
        let (failed, action) = retryable(connection, run_id)?;
        let run = pending_run(&action, failed.scheduled_for_ms, RunTrigger::Retry, Some(failed.id), app);
        insert_run(connection, &run)?;
        Ok((run, action))
    })?;
    start_run(app, library_id, database, &run, &action.name, action.kind, &when_of(&action), &action.prompt, false)?;
    Ok(run)
}

/// Runs an action now, out of its schedule and without changing it.
fn run_now(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, action_id: &str) -> AiActionsResult<AiActionRun> {
    ensure_telegram(app, library_id)?;
    let (run, action) = write(app, database, |connection| {
        let action = load_action(connection, action_id)?;
        let run = pending_run(&action, now_ms(), RunTrigger::Manual, None, app);
        insert_run(connection, &run)?;
        Ok((run, action))
    })?;
    start_run(app, library_id, database, &run, &action.name, action.kind, &when_of(&action), &action.prompt, false)?;
    Ok(run)
}

pub async fn ai_action_create(app: AppHandle, context: AiActionsContext, input: AiActionInput) -> AiActionsResult<SavedAction> {
    blocking(move || {
        let database = context.database()?;
        let action = create_action(&app, &context.library_id, &database, &input, true)?;
        Ok(SavedAction { action_id: action.id })
    })
    .await
}

pub async fn ai_action_update(
    app: AppHandle,
    context: AiActionsContext,
    action_id: String,
    input: AiActionInput,
) -> AiActionsResult<SavedAction> {
    blocking(move || {
        let database = context.database()?;
        update_action(&app, &context.library_id, &database, &action_id, &input, None)?;
        Ok(SavedAction { action_id })
    })
    .await
}

pub async fn ai_action_set_enabled(app: AppHandle, context: AiActionsContext, action_id: String, enabled: bool) -> AiActionsResult<()> {
    blocking(move || {
        let database = context.database()?;
        set_action_enabled(&app, &context.library_id, &database, &action_id, enabled).map(|_| ())
    })
    .await
}

pub async fn ai_action_delete(app: AppHandle, context: AiActionsContext, action_id: String) -> AiActionsResult<()> {
    blocking(move || {
        let database = context.database()?;
        delete_action(&app, &context.library_id, &database, &action_id).map(|_| ())
    })
    .await
}

pub async fn ai_action_runs(app: AppHandle, context: AiActionsContext, action_id: String) -> AiActionsResult<Vec<RunRow>> {
    blocking(move || {
        let database = context.database()?;
        let zone = time_zone(&library_zone(&app, &context.library_id));
        let runs = read(&app, &database, |connection| query_runs(connection, "action_id = ?1", &action_id))?;
        Ok(run_history(&action_id, &runs, now_ms(), &zone))
    })
    .await
}

pub async fn ai_action_retry(app: AppHandle, context: AiActionsContext, run_id: String) -> AiActionsResult<StartedRun> {
    blocking(move || {
        let database = context.database()?;
        let run = retry_run(&app, &context.library_id, &database, &run_id)?;
        Ok(StartedRun { run_id: run.id })
    })
    .await
}

/// Runs a prompt once without saving it; its answer reaches Telegram with
/// the «[Prueba]» prefix.
pub async fn ai_action_test(app: AppHandle, context: AiActionsContext, input: AiActionInput) -> AiActionsResult<StartedRun> {
    blocking(move || {
        let database = context.database()?;
        let mut errors = Vec::new();
        if input.prompt.trim().is_empty() {
            errors.push(FieldError { field: "prompt".into(), message: "Escribí el prompt para probarlo.".into() });
        }
        let Some(kind) = input.kind else {
            errors.push(FieldError { field: "kind".into(), message: "Elegí el tipo de acción.".into() });
            return Err(AiActionsError::validation(errors));
        };
        if !errors.is_empty() {
            return Err(AiActionsError::validation(errors));
        }
        ensure_telegram(&app, &context.library_id)?;
        let name = Some(input.name.trim()).filter(|name| !name.is_empty()).unwrap_or(TEST_NAME).to_string();
        let now = now_ms();
        let run = AiActionRun {
            id: Uuid::new_v4().to_string(),
            action_id: None,
            action_name: name.clone(),
            kind,
            scheduled_for_ms: now,
            created_at_ms: now,
            started_at_ms: None,
            finished_at_ms: None,
            status: RunStatus::Pending,
            error: None,
            output_summary: None,
            trigger: RunTrigger::Test,
            retry_of: None,
            runner: Some(instance_id(&app)),
        };
        write(&app, &database, |connection| insert_run(connection, &run))?;
        let zone = library_zone(&app, &context.library_id);
        let when = preview(&input, &zone, None, now).when.unwrap_or_else(|| kind.label().to_string());
        start_run(&app, &context.library_id, &database, &run, &name, kind, &when, input.prompt.trim(), true)?;
        Ok(StartedRun { run_id: run.id })
    })
    .await
}

fn when_of(action: &AiAction) -> String {
    let zone = time_zone(&action.time_zone);
    schedule_label(&action.schedule, &zone, local_date(&zone, now_ms()))
}

// ---------------------------------------------------------------------------
// Agent tools
// ---------------------------------------------------------------------------

pub(crate) use crate::backend::ai_actions::tools::{is_ai_action_tool, is_ai_action_write_tool};

/// A tool's argument error, as the form's validation error.
fn argument_error(error: crate::backend::BackendError) -> AiActionsError {
    let code = if error.code == crate::backend::BackendErrorCode::NotFound {
        AiActionsErrorCode::NotFound
    } else {
        AiActionsErrorCode::Validation
    };
    AiActionsError::new(code, error.message)
}

/// The error a tool returns to the model: the form's fields in one message.
pub(crate) fn tool_error(error: AiActionsError) -> crate::backend::BackendError {
    use crate::backend::{BackendError, BackendErrorCode};
    match error.code {
        AiActionsErrorCode::Validation if !error.fields.is_empty() => BackendError::invalid_input(tools::field_errors_message(&error.fields)),
        AiActionsErrorCode::Validation => BackendError::invalid_input(error.message),
        AiActionsErrorCode::NotFound => BackendError::new(BackendErrorCode::NotFound, error.message, false),
        AiActionsErrorCode::Forbidden => BackendError::new(BackendErrorCode::Forbidden, error.message, false),
        AiActionsErrorCode::Unavailable => BackendError::new(BackendErrorCode::ProviderUnavailable, error.message, true),
        AiActionsErrorCode::Storage => BackendError::new(BackendErrorCode::Storage, error.message, true),
    }
}

fn named<'a>(actions: &'a [AiAction], arguments: &Value) -> AiActionsResult<&'a AiAction> {
    let reference = tools::required_text(arguments, "action").map_err(argument_error)?;
    tools::resolve_action(actions, &reference).map_err(argument_error)
}

fn enabled_of(arguments: &Value) -> AiActionsResult<Option<bool>> {
    tools::enabled_argument(arguments).map_err(argument_error)
}

/// The confirmation of a write tool: the call is read and checked as its
/// execution will do it, without saving anything.
pub(crate) fn preview_tool(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, name: &str, arguments: &Value) -> AiActionsResult<String> {
    let now = now_ms();
    let actions = read(app, database, load_actions)?;
    match name {
        "create_ai_action" => {
            let input = tools::input_from_arguments(arguments, tools::empty_input()).map_err(argument_error)?;
            let zone_name = library_zone(app, library_id);
            let valid = validate_input(&input, &time_zone(&zone_name), now, InputOrigin::default()).map_err(AiActionsError::validation)?;
            Ok(tools::create_summary(&valid, &zone_name, enabled_of(arguments)?.unwrap_or(true), now))
        }
        "update_ai_action" => {
            let action = named(&actions, arguments)?;
            let input = tools::input_from_arguments(arguments, tools::stored_input(action)).map_err(argument_error)?;
            let valid = validate_input(&input, &time_zone(&action.time_zone), now, origin_of(action)).map_err(AiActionsError::validation)?;
            Ok(tools::update_summary(action, &valid, enabled_of(arguments)?, now))
        }
        "set_ai_action_enabled" => {
            let action = named(&actions, arguments)?;
            let enabled = enabled_of(arguments)?.ok_or_else(|| AiActionsError::new(AiActionsErrorCode::Validation, "Falta enabled."))?;
            Ok(tools::enabled_summary(action, enabled))
        }
        "delete_ai_action" => Ok(tools::delete_summary(named(&actions, arguments)?)),
        "run_ai_action_now" => {
            ensure_telegram(app, library_id)?;
            Ok(tools::run_now_summary(named(&actions, arguments)?))
        }
        "retry_ai_action_run" => {
            ensure_telegram(app, library_id)?;
            let run_id = tools::required_text(arguments, "runId").map_err(argument_error)?;
            let (_, action) = read(app, database, |connection| retryable(connection, &run_id))?;
            Ok(tools::retry_summary(&action.name))
        }
        _ => Err(AiActionsError::new(AiActionsErrorCode::Validation, "La herramienta no es de Acciones IA.")),
    }
}

/// Runs an AI action tool: a read, or a write the Owner confirmed.
pub(crate) fn execute_tool(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, name: &str, arguments: &Value) -> AiActionsResult<Value> {
    let now = now_ms();
    let zone = time_zone(&library_zone(app, library_id));
    let actions = read(app, database, load_actions)?;
    let saved = |action: &AiAction, what: &str| {
        json!({ "id": action.id, "name": action.name, what: true, "enabled": action.enabled, "when": when_of(action) })
    };
    match name {
        "list_ai_actions" => {
            let runs = read(app, database, |connection| query_runs(connection, "scheduled_for >= ?1", now - RECENT_RUNS_MS))?;
            let dashboard = build_dashboard(&actions, &runs, now, &zone, DashboardFilter::All, "");
            Ok(tools::list_view(&actions, &dashboard, now, &zone))
        }
        "get_ai_action" => {
            let action = named(&actions, arguments)?;
            let runs = read(app, database, |connection| query_runs(connection, "action_id = ?1", &action.id))?;
            Ok(tools::detail_view(action, &run_history(&action.id, &runs, now, &zone), now, &zone))
        }
        "create_ai_action" => {
            let input = tools::input_from_arguments(arguments, tools::empty_input()).map_err(argument_error)?;
            let action = create_action(app, library_id, database, &input, enabled_of(arguments)?.unwrap_or(true))?;
            Ok(saved(&action, "created"))
        }
        "update_ai_action" => {
            let action = named(&actions, arguments)?;
            let input = tools::input_from_arguments(arguments, tools::stored_input(action)).map_err(argument_error)?;
            let action = update_action(app, library_id, database, &action.id, &input, enabled_of(arguments)?)?;
            Ok(saved(&action, "updated"))
        }
        "set_ai_action_enabled" => {
            let action = named(&actions, arguments)?;
            let enabled = enabled_of(arguments)?.ok_or_else(|| AiActionsError::new(AiActionsErrorCode::Validation, "Falta enabled."))?;
            let action = set_action_enabled(app, library_id, database, &action.id, enabled)?;
            Ok(saved(&action, "updated"))
        }
        "delete_ai_action" => {
            let action = delete_action(app, library_id, database, &named(&actions, arguments)?.id)?;
            Ok(json!({ "id": action.id, "name": action.name, "deleted": true }))
        }
        "run_ai_action_now" => {
            let run = run_now(app, library_id, database, &named(&actions, arguments)?.id)?;
            Ok(json!({ "runId": run.id, "name": run.action_name, "queued": true, "note": "La respuesta llega por Telegram cuando termine este turno." }))
        }
        "retry_ai_action_run" => {
            let run_id = tools::required_text(arguments, "runId").map_err(argument_error)?;
            let run = retry_run(app, library_id, database, &run_id)?;
            Ok(json!({ "runId": run.id, "name": run.action_name, "queued": true }))
        }
        _ => Err(AiActionsError::new(AiActionsErrorCode::Validation, "La herramienta no es de Acciones IA.")),
    }
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

/// Runs this process queued and has not finished yet.
fn in_hand() -> &'static Mutex<HashSet<String>> {
    static IN_HAND: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_HAND.get_or_init(Default::default)
}

/// Hands a stored pending run to the Telegram worker. A run the worker
/// cannot take fails at once.
#[allow(clippy::too_many_arguments)]
fn start_run(
    app: &AppHandle,
    library_id: &str,
    database: &LibraryDatabaseContext,
    run: &AiActionRun,
    name: &str,
    kind: AiActionKind,
    when: &str,
    prompt: &str,
    test: bool,
) -> AiActionsResult<()> {
    if let Ok(mut runs) = in_hand().lock() {
        runs.insert(run.id.clone());
    }
    emit(app, RUN_UPDATED_EVENT, library_id);
    // A test of a form not saved yet has no earlier runs.
    let recent_messages = match &run.action_id {
        Some(action_id) => read(app, database, |connection| query_runs(connection, "action_id = ?1", action_id))
            .map(|runs| crate::backend::ai_actions::prompts::recent_messages(&runs))
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let job = ActionJob {
        run_id: run.id.clone(),
        prompt: ScheduledActionPrompt { name: name.to_string(), kind, when: when.to_string(), test, recent_messages },
        request: action_request(name, prompt, &crate::local_time::local_now().1, test),
    };
    if enqueue_action(app, library_id, job) == ActionEnqueue::Queued {
        return Ok(());
    }
    finish_run(app, library_id, database, &run.id, Err(telegram_unavailable().message));
    Err(telegram_unavailable())
}

/// The worker started the run.
pub(crate) fn mark_running(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, run_id: &str) {
    let result = write(app, database, |connection| {
        let mut run = load_run(connection, run_id)?;
        run.status = RunStatus::Running;
        run.started_at_ms = Some(now_ms());
        update_run(connection, &run)
    });
    match result {
        Ok(()) => emit(app, RUN_UPDATED_EVENT, library_id),
        Err(error) => log::error!("[notia:ai-actions] no se pudo marcar el inicio de una ejecución: {:?}", error.code),
    }
}

/// The worker finished the run: its summary, or the error to show.
pub(crate) fn finish_run(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, run_id: &str, outcome: Result<String, String>) {
    if let Ok(mut runs) = in_hand().lock() {
        runs.remove(run_id);
    }
    let result = write(app, database, |connection| {
        let mut run = load_run(connection, run_id)?;
        let now = now_ms();
        run.started_at_ms = run.started_at_ms.or(Some(now));
        run.finished_at_ms = Some(now);
        match outcome {
            Ok(summary) => {
                run.status = RunStatus::Success;
                run.output_summary = Some(summary);
                run.error = None;
            }
            Err(error) => {
                run.status = RunStatus::Failed;
                run.error = Some(error);
            }
        }
        update_run(connection, &run)
    });
    match result {
        Ok(()) => emit(app, RUN_UPDATED_EVENT, library_id),
        Err(error) => log::error!("[notia:ai-actions] no se pudo guardar el resultado de una ejecución: {:?}", error.code),
    }
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

static SERVER_INSTANCE: AtomicBool = AtomicBool::new(false);

/// Called by the headless server before its startup hooks: it takes the
/// clock over from the other instances. Android and iOS have no server.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(crate) fn run_as_server() {
    SERVER_INSTANCE.store(true, Ordering::SeqCst);
}

fn instance_kind() -> InstanceKind {
    if cfg!(target_os = "android") {
        InstanceKind::Android
    } else if SERVER_INSTANCE.load(Ordering::SeqCst) {
        InstanceKind::Server
    } else {
        InstanceKind::Desktop
    }
}

fn instance_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|directory| directory.join(STATE_DIRECTORY).join("instance.json"))
}

#[derive(Serialize, Deserialize)]
struct InstanceFile {
    id: String,
}

/// This installation's id, kept in its data folder (the window and the
/// headless server of one folder never run at once).
fn instance_id(app: &AppHandle) -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let file = instance_file(app);
        if let Some(id) = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str::<InstanceFile>(&text).ok())
            .map(|stored| stored.id)
            .filter(|id| !id.trim().is_empty())
        {
            return id;
        }
        let id = Uuid::new_v4().to_string();
        if let Some(file) = file {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string(&InstanceFile { id: id.clone() }) {
                let _ = std::fs::write(file, text);
            }
        }
        id
    })
    .clone()
}

pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("ai-actions")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-ai-actions".into()).spawn(move || {
                std::thread::sleep(FIRST_TICK_DELAY);
                let mut failing = false;
                loop {
                    match tick(&app) {
                        Ok(()) => failing = false,
                        Err(error) => {
                            // A lasting failure is logged once.
                            if !failing {
                                log::error!("[notia:ai-actions] el reloj de las acciones falló: {:?} {}", error.code, error.message);
                            }
                            failing = true;
                        }
                    }
                    std::thread::sleep(TICK);
                }
            });
            Ok(())
        })
        .build()
}

/// What a tick decided, to run after the database is saved.
struct Due {
    run: AiActionRun,
    action: AiAction,
}

fn tick(app: &AppHandle) -> AiActionsResult<()> {
    // Only the computer that runs the library's services runs the actions.
    if !crate::device_role::runs_services(app) {
        return Ok(());
    }
    let Some(library) = crate::library_catalog::selected_library(app) else {
        return Ok(());
    };
    if app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library.id).is_err() {
        return Ok(());
    }
    // Only where the bot runs: the answers can only go out from there.
    if !crate::telegram_worker::bot_runs_for(app, &library.id) {
        return Ok(());
    }
    let database = LibraryDatabaseContext { library_path: library.path.clone(), android_directory_uri: library.android_tree_uri.clone() };
    let me = instance_id(app);
    let kind = instance_kind();
    let now = now_ms();

    // Reading first keeps a tick with nothing to do from writing (on
    // Android, each write copies the database back through SAF).
    let (lease, actions, runs, last_check, seeded) = read(app, &database, |connection| {
        let lease = meta(connection, LEASE_KEY)?.and_then(|text| serde_json::from_str::<SchedulerLease>(&text).ok());
        let last_check = meta(connection, LAST_CHECK_KEY)?.and_then(|text| text.parse::<i64>().ok());
        let seeded = meta(connection, HOURLY_REVIEW_SEEDED_KEY)?.is_some();
        Ok((lease, load_actions(connection)?, planning_runs(connection, now)?, last_check, seeded))
    })?;
    let Some(claimed) = claim_lease(lease.as_ref(), &me, kind, now) else {
        return Ok(());
    };
    let held = in_hand().lock().map(|runs| runs.clone()).unwrap_or_default();
    let interrupted = interrupted_runs(&runs, &me, &held, now).into_iter().cloned().collect::<Vec<_>>();
    let mut plans = Vec::new();
    for action in &actions {
        for plan in plan_action(action, &runs, last_check, now) {
            plans.push((action.clone(), plan));
        }
    }
    let renew = lease_needs_renewal(lease.as_ref(), &me, now);
    if plans.is_empty() && interrupted.is_empty() && !renew && seeded {
        return Ok(());
    }

    let (due, seeded_now) = write(app, &database, |connection| {
        let seeded_now = ensure_defaults(app, &library.id, connection, now)?;
        // Another instance may have taken the lease since the read.
        let current = meta(connection, LEASE_KEY)?.and_then(|text| serde_json::from_str::<SchedulerLease>(&text).ok());
        if claim_lease(current.as_ref(), &me, kind, now).is_none() {
            return Ok((Vec::new(), seeded_now));
        }
        if renew {
            set_meta(connection, LEASE_KEY, &serde_json::to_string(&claimed).unwrap_or_default())?;
            set_meta(connection, LAST_CHECK_KEY, &now.to_string())?;
            connection.execute("DELETE FROM ai_action_runs WHERE created_at < ?1", [now - RUN_RETENTION_MS])?;
        }
        for run in &interrupted {
            let mut run = run.clone();
            run.status = RunStatus::Failed;
            run.error = Some(INTERRUPTED.to_string());
            run.finished_at_ms = Some(now);
            update_run(connection, &run)?;
        }
        let mut due = Vec::new();
        for (action, plan) in plans {
            let (scheduled_for_ms, status, error) = match plan {
                Planned::Run { scheduled_for_ms } => (scheduled_for_ms, RunStatus::Pending, None),
                Planned::Skip { scheduled_for_ms, reason } => (scheduled_for_ms, RunStatus::Skipped, Some(reason.to_string())),
            };
            let run = AiActionRun {
                id: Uuid::new_v4().to_string(),
                action_id: Some(action.id.clone()),
                action_name: action.name.clone(),
                kind: action.kind,
                scheduled_for_ms,
                created_at_ms: now,
                started_at_ms: None,
                finished_at_ms: (status == RunStatus::Skipped).then_some(now),
                status,
                error,
                output_summary: None,
                trigger: RunTrigger::Scheduled,
                retry_of: None,
                runner: Some(me.clone()),
            };
            if insert_run(connection, &run)? && status == RunStatus::Pending {
                due.push(Due { run, action });
            }
        }
        Ok((due, seeded_now))
    })?;
    if seeded_now {
        emit(app, ACTION_CHANGED_EVENT, &library.id);
    }
    if !interrupted.is_empty() {
        emit(app, RUN_UPDATED_EVENT, &library.id);
    }
    for Due { run, action } in due {
        // A run the worker cannot take is already marked failed.
        let _ = start_run(app, &library.id, &database, &run, &action.name, action.kind, &when_of(&action), &action.prompt, false);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let connection = Connection::open_in_memory().expect("memory");
        crate::database::migrate(&connection).expect("schema");
        connection
    }

    #[test]
    fn actions_and_runs_survive_a_round_trip() {
        let connection = memory();
        let review = core::hourly_review_action("a1".into(), "America/Argentina/Buenos_Aires".into(), true, 1_000);
        save_action(&connection, &review).expect("save");
        assert_eq!(load_actions(&connection).expect("load"), vec![review.clone()]);
        let run = AiActionRun {
            id: "r1".into(),
            action_id: Some("a1".into()),
            action_name: review.name.clone(),
            kind: review.kind,
            scheduled_for_ms: 5_000,
            created_at_ms: 5_000,
            started_at_ms: None,
            finished_at_ms: None,
            status: RunStatus::Pending,
            error: None,
            output_summary: None,
            trigger: RunTrigger::Scheduled,
            retry_of: None,
            runner: Some("me".into()),
        };
        assert!(insert_run(&connection, &run).expect("insert"));
        // One scheduled run per occurrence, whoever tries again.
        assert!(!insert_run(&connection, &AiActionRun { id: "r2".into(), ..run.clone() }).expect("duplicate"));
        // A retry of the same occurrence is another run.
        assert!(insert_run(&connection, &AiActionRun { id: "r3".into(), trigger: RunTrigger::Retry, ..run.clone() }).expect("retry"));
        let mut done = run.clone();
        done.status = RunStatus::Success;
        done.output_summary = Some("Listo".into());
        update_run(&connection, &done).expect("update");
        assert_eq!(load_run(&connection, "r1").expect("run"), done);
        // Deleting the action keeps its history.
        connection.execute("DELETE FROM ai_actions WHERE id='a1'", []).expect("delete");
        assert_eq!(query_runs(&connection, "action_id = ?1", "a1").expect("history").len(), 2);
    }

    #[test]
    fn the_preview_explains_a_valid_form_and_lists_its_errors() {
        let zone = time_zone("America/Argentina/Buenos_Aires");
        let date = crate::backend::ai_actions::schedule::parse_date("2026-09-28").expect("date");
        let now = crate::backend::ai_actions::schedule::local_instant(&zone, date, 13 * 60 + 26).expect("13:26");
        let mut input = AiActionInput {
            kind: Some(AiActionKind::Recurring),
            name: "Gastos".into(),
            prompt: "Preguntame por gastos".into(),
            every: Some("3".into()),
            unit: Some(core::RepeatUnit::Hours),
            weekdays: vec![4, 0, 1, 2, 3],
            from: Some("09:00".into()),
            to: Some("21:00".into()),
            ..AiActionInput::default()
        };
        let valid = preview(&input, "America/Argentina/Buenos_Aires", None, now);
        assert!(valid.errors.is_empty());
        assert!(valid.summary.contains("de lunes a viernes"));
        assert_eq!(valid.when.as_deref(), Some("Cada 3 h · 09 a 21 h · Lun a Vie"));
        assert_eq!(valid.upcoming, ["Hoy · 15:00", "Hoy · 18:00", "Hoy · 21:00"]);
        input.weekdays.clear();
        let invalid = preview(&input, "America/Argentina/Buenos_Aires", None, now);
        assert_eq!(invalid.errors.iter().map(|error| error.field.as_str()).collect::<Vec<_>>(), ["weekdays"]);
        assert!(invalid.upcoming.is_empty());
    }

    #[test]
    fn an_edit_starts_a_new_schedule_and_a_reactivation_now() {
        let review = core::hourly_review_action("a1".into(), "America/Argentina/Buenos_Aires".into(), false, 1_000);
        let same = ValidAction { name: "Otro nombre".into(), prompt: review.prompt.clone(), kind: review.kind, schedule: review.schedule.clone() };
        let renamed = edited(&review, same.clone(), None, 5_000);
        assert_eq!((renamed.name.as_str(), renamed.active_since_ms, renamed.enabled), ("Otro nombre", 1_000, false));
        let resumed = edited(&review, same, Some(true), 6_000);
        assert_eq!((resumed.enabled, resumed.active_since_ms, resumed.updated_at_ms), (true, 6_000, 6_000));
        let mut schedule = review.schedule.clone();
        if let AiSchedule::Recurring { rule } = &mut schedule {
            rule.every = 2;
        }
        let rescheduled = edited(&review, ValidAction { name: review.name.clone(), prompt: review.prompt.clone(), kind: review.kind, schedule }, None, 7_000);
        assert_eq!(rescheduled.active_since_ms, 7_000);
        let created = new_action(ValidAction { name: "N".into(), prompt: "P".into(), kind: AiActionKind::Reminder, schedule: AiSchedule::Once { at_ms: 9_000 } }, "UTC".into(), false, 8_000);
        assert_eq!((created.enabled, created.active_since_ms, created.builtin), (false, 8_000, None));
    }

    #[test]
    fn meta_values_are_replaced() {
        let connection = memory();
        assert_eq!(meta(&connection, "k").expect("empty"), None);
        set_meta(&connection, "k", "1").expect("set");
        set_meta(&connection, "k", "2").expect("replace");
        assert_eq!(meta(&connection, "k").expect("value").as_deref(), Some("2"));
    }
}
