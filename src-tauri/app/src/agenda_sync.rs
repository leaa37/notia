//! Two-way sync between the Agenda of Notia and Google Calendar, every five
//! minutes, over every account of the open library that granted the calendar
//! permission. Each account's events come into the Agenda; events made in
//! Notia go to one account (the first personal one, else the first).
//!
//! `agenda_google_links` pairs each Agenda event with its Google event and
//! keeps both as they were at the last sync, so the sync knows which side
//! changed. When both changed, Notia wins. All-day events are left out: the
//! Agenda only has timed blocks.

use std::collections::{HashMap, HashSet};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Local, NaiveDate, SecondsFormat, TimeZone, Timelike};
use reqwest::Method;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::agenda::{
    query_events, with_transaction, AgendaContext, AgendaPriority, AgendaResult, EventRecord, DAY_MINUTES,
    MAX_EVENT_TITLE_CHARS, SLOT_MINUTES, UNTITLED_EVENT,
};
use crate::agenda_tools::AGENDA_DATA_CHANGED_EVENT;
use crate::agenda_view::date_key;
use crate::backend::mail_accounts::MailAccountType;
use crate::backend::mail_tools::{MailAccountRef, CALENDAR_API};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Emitter};
use crate::mail_tools::{calendar_accounts, calendar_session, CalendarSession};

const SYNC_INTERVAL: StdDuration = StdDuration::from_secs(5 * 60);
/// The first sync waits for the app to open its library.
const FIRST_SYNC_DELAY: StdDuration = StdDuration::from_secs(30);
/// Days before today and after today that the sync covers.
const PAST_DAYS: i64 = 30;
const FUTURE_DAYS: i64 = 365;
const PAGE_SIZE: u32 = 250;
const MAX_PAGES: usize = 20;
/// Linked events outside the listed window asked one by one, per account and sync.
const MAX_SINGLE_READS: usize = 50;
/// The library owner, who connects the Google accounts.
const OWNER: &str = "user-owner";

pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("agenda-google-sync")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-agenda-google-sync".into()).spawn(move || {
                std::thread::sleep(FIRST_SYNC_DELAY);
                loop {
                    sync_selected_library(&app);
                    std::thread::sleep(SYNC_INTERVAL);
                }
            });
            Ok(())
        })
        .build()
}

fn sync_selected_library(app: &AppHandle) {
    // Only the computer that runs the library's services syncs with Google.
    if !crate::device_role::runs_services(app) {
        return;
    }
    let Some(library) = crate::library_catalog::selected_library(app) else {
        return;
    };
    let accounts = calendar_accounts(app, &library.id);
    if accounts.is_empty() {
        return;
    }
    let context = AgendaContext {
        library_path: library.path.clone(),
        android_directory_uri: library.android_tree_uri.clone(),
        actor_library_user_id: OWNER.to_string(),
    };
    let target = target_account(&accounts).cloned();
    let mut changed = false;
    for (index, account) in accounts.iter().enumerate() {
        let is_target = target.as_ref() == Some(account);
        match sync_account(app, &library.id, &context, account, is_target) {
            Ok(account_changed) => changed |= account_changed,
            // Neither the address nor the error body: only which account failed and why.
            Err(error) => log::error!(
                "[notia:agenda-sync] la cuenta {} ({}) no se sincronizó: {:?}",
                index + 1,
                account.account_type.id(),
                error.code
            ),
        }
    }
    if changed {
        if let Err(error) = app.emit(AGENDA_DATA_CHANGED_EVENT, ()) {
            log::error!("[notia:agenda-sync] evento de cambio no emitido: {error}");
        }
    }
}

/// The account events made in Notia go to: the first personal one, else the first.
fn target_account(accounts: &[MailAccountRef]) -> Option<&MailAccountRef> {
    accounts
        .iter()
        .find(|account| account.account_type == MailAccountType::Personal)
        .or_else(|| accounts.first())
}

// ---------- Both sides ----------

/// Date and minutes of an event on the Agenda's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timing {
    date: NaiveDate,
    start_minute: u16,
    end_minute: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoogleEvent {
    id: String,
    ical_uid: Option<String>,
    updated: String,
    cancelled: bool,
    title: String,
    /// `None` for all-day events.
    timing: Option<Timing>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    account: String,
    google_id: String,
    event_id: String,
    ical_uid: Option<String>,
    google_updated: String,
    fingerprint: String,
}

/// What the sync compares of an Agenda event: what Google also has.
fn fingerprint(event: &EventRecord) -> String {
    format!("{}|{}|{}|{}", date_key(event.date), event.start_minute, event.end_minute, event.title)
}

/// Google event ids are base32hex, plus `_` and the instant of a recurring
/// instance; anything else is not put in a URL.
fn valid_google_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 1024 && id.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-'))
}

fn clean_title(value: &str) -> String {
    let spaced: String = value.chars().map(|char| if char.is_control() { ' ' } else { char }).collect();
    let title: String = spaced.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_EVENT_TITLE_CHARS).collect();
    if title.is_empty() {
        UNTITLED_EVENT.to_string()
    } else {
        title
    }
}

fn minute_of_day<Tz: TimeZone>(moment: &DateTime<Tz>) -> u16 {
    (moment.hour() * 60 + moment.minute()) as u16
}

/// The Agenda's blocks for a timed event, in `zone`: the start rounds down
/// and the end up to the 15-minute grid, and an event that goes past
/// midnight ends at 24:00 of the day it starts.
fn timing_in<Tz: TimeZone>(start: &str, end: &str, zone: &Tz) -> Option<Timing> {
    let start = DateTime::parse_from_rfc3339(start).ok()?.with_timezone(zone);
    let end = DateTime::parse_from_rfc3339(end).ok()?.with_timezone(zone);
    if end <= start {
        return None;
    }
    let date = start.date_naive();
    let start_minute = minute_of_day(&start) / SLOT_MINUTES * SLOT_MINUTES;
    let end_minute = if end.date_naive() > date { DAY_MINUTES } else { minute_of_day(&end).div_ceil(SLOT_MINUTES) * SLOT_MINUTES };
    let end_minute = end_minute.max(start_minute + SLOT_MINUTES).min(DAY_MINUTES);
    Some(Timing { date, start_minute, end_minute })
}

fn google_event_in<Tz: TimeZone>(value: &Value, zone: &Tz) -> Option<GoogleEvent> {
    let text = |pointer: &str| value.pointer(pointer).and_then(Value::as_str);
    let id = text("/id").filter(|id| valid_google_id(id))?;
    let timing = match (text("/start/dateTime"), text("/end/dateTime")) {
        (Some(start), Some(end)) => timing_in(start, end, zone),
        _ => None,
    };
    Some(GoogleEvent {
        id: id.to_string(),
        ical_uid: text("/iCalUID").map(str::to_string),
        updated: text("/updated").unwrap_or_default().to_string(),
        cancelled: text("/status") == Some("cancelled"),
        title: clean_title(text("/summary").unwrap_or_default()),
        timing,
    })
}

/// A local moment as RFC 3339 with its offset.
fn moment_in<Tz: TimeZone>(date: NaiveDate, minute: u16, zone: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let naive = date.and_hms_opt(0, 0, 0)? + Duration::minutes(i64::from(minute));
    zone.from_local_datetime(&naive).earliest().map(|moment| moment.to_rfc3339_opts(SecondsFormat::Secs, false))
}

/// The Google event for an Agenda event.
fn google_body_in<Tz: TimeZone>(event: &EventRecord, zone: &Tz) -> Option<Value>
where
    Tz::Offset: std::fmt::Display,
{
    Some(json!({
        "summary": event.title,
        "start": { "dateTime": moment_in(event.date, event.start_minute, zone)? },
        "end": { "dateTime": moment_in(event.date, event.end_minute, zone)? },
    }))
}

// ---------- The plan ----------

/// What one sync of one account does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// A Notia event that is in no calendar yet (target account only).
    CreateInGoogle { event_id: String },
    /// Notia changed the event: Google takes Notia's version.
    UpdateGoogle { google_id: String, event_id: String },
    /// Notia deleted the event.
    DeleteInGoogle { google_id: String },
    /// Google deleted an event Notia changed: Notia wins and makes it again.
    RecreateInGoogle { google_id: String, event_id: String },
    /// A Google event Notia does not have.
    Import { google_id: String },
    /// A Google event identical to an unlinked Notia event (made on both
    /// sides before the first sync): they become one.
    Adopt { google_id: String, event_id: String },
    /// Google changed the event and Notia did not.
    UpdateNotia { google_id: String, event_id: String },
    /// Google deleted the event, or made it all-day, and Notia did not change it.
    DeleteInNotia { google_id: String, event_id: String },
    /// Both sides deleted the event.
    ForgetLink { google_id: String },
}

struct Snapshot<'a> {
    /// Agenda events of the window and every linked one, by id.
    events: &'a HashMap<String, EventRecord>,
    /// Links of every account.
    links: &'a [Link],
    /// Agenda events of the window, oldest first.
    window: &'a [String],
}

fn plan(account: &str, is_target: bool, snapshot: &Snapshot, google: &HashMap<String, GoogleEvent>) -> Vec<Action> {
    let mut actions = Vec::new();
    let own_links: Vec<&Link> = snapshot.links.iter().filter(|link| link.account == account).collect();
    for link in &own_links {
        let Some(remote) = google.get(&link.google_id) else {
            continue;
        };
        let google_id = link.google_id.clone();
        let Some(event) = snapshot.events.get(&link.event_id) else {
            actions.push(if remote.cancelled { Action::ForgetLink { google_id } } else { Action::DeleteInGoogle { google_id } });
            continue;
        };
        let event_id = event.id.clone();
        let notia_changed = fingerprint(event) != link.fingerprint;
        let google_changed = remote.updated != link.google_updated;
        let action = if remote.cancelled {
            if notia_changed {
                Action::RecreateInGoogle { google_id, event_id }
            } else {
                Action::DeleteInNotia { google_id, event_id }
            }
        } else if notia_changed {
            Action::UpdateGoogle { google_id, event_id }
        } else if !google_changed {
            continue;
        } else if remote.timing.is_some() {
            Action::UpdateNotia { google_id, event_id }
        } else {
            Action::DeleteInNotia { google_id, event_id }
        };
        actions.push(action);
    }
    let linked_ids: HashSet<&str> = own_links.iter().map(|link| link.google_id.as_str()).collect();
    // The instances of a recurring event share their iCalUID: the day and
    // hour tell them apart.
    let meeting = |uid: &str, date: NaiveDate, start_minute: u16| format!("{uid}|{date}|{start_minute}");
    let linked_meetings: HashSet<String> = snapshot
        .links
        .iter()
        .filter_map(|link| {
            let event = snapshot.events.get(&link.event_id)?;
            Some(meeting(link.ical_uid.as_deref()?, event.date, event.start_minute))
        })
        .collect();
    let linked_events: HashSet<&str> = snapshot.links.iter().map(|link| link.event_id.as_str()).collect();
    let mut unlinked: Vec<&EventRecord> = snapshot
        .window
        .iter()
        .filter(|id| !linked_events.contains(id.as_str()))
        .filter_map(|id| snapshot.events.get(id))
        .collect();
    let mut imported_meetings: HashSet<String> = HashSet::new();
    let mut remote: Vec<&GoogleEvent> = google.values().collect();
    remote.sort_by(|left, right| left.id.cmp(&right.id));
    for event in remote {
        let Some(timing) = event.timing.filter(|_| !event.cancelled && !linked_ids.contains(event.id.as_str())) else {
            continue;
        };
        // The same meeting in two accounts comes in once.
        if let Some(uid) = event.ical_uid.as_deref() {
            let key = meeting(uid, timing.date, timing.start_minute);
            if linked_meetings.contains(&key) || !imported_meetings.insert(key) {
                continue;
            }
        }
        let same = unlinked.iter().position(|notia| {
            timing == Timing { date: notia.date, start_minute: notia.start_minute, end_minute: notia.end_minute }
                && notia.title.to_lowercase() == event.title.to_lowercase()
        });
        actions.push(match same {
            Some(index) => Action::Adopt { google_id: event.id.clone(), event_id: unlinked.remove(index).id.clone() },
            None => Action::Import { google_id: event.id.clone() },
        });
    }
    if is_target {
        actions.extend(unlinked.into_iter().map(|event| Action::CreateInGoogle { event_id: event.id.clone() }));
    }
    actions
}

// ---------- Running it ----------

/// A database change that follows the Google calls.
enum Write {
    InsertEvent(EventRecord),
    UpdateEvent(EventRecord),
    DeleteEvent(String),
    SaveLink(Link),
    DeleteLink { account: String, google_id: String },
}

fn events_url(from: NaiveDate, to: NaiveDate, page_token: Option<&str>) -> Option<String> {
    let bound = |date: NaiveDate| {
        Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
            .earliest()
            .map(|moment| moment.with_timezone(&chrono::Utc).to_rfc3339_opts(SecondsFormat::Secs, true))
    };
    let mut url = format!(
        "{CALENDAR_API}/events?singleEvents=true&showDeleted=true&maxResults={PAGE_SIZE}&timeMin={}&timeMax={}&fields=items(id,status,updated,iCalUID,summary,start,end),nextPageToken",
        bound(from)?,
        bound(to + Duration::days(1))?
    );
    if let Some(token) = page_token {
        url.push_str("&pageToken=");
        url.push_str(&encode(token));
    }
    Some(url)
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn event_url(google_id: &str) -> String {
    format!("{CALENDAR_API}/events/{google_id}")
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    crate::host::async_runtime::block_on(future)
}

fn list_window(session: &CalendarSession, from: NaiveDate, to: NaiveDate) -> Result<HashMap<String, GoogleEvent>, BackendError> {
    let mut events = HashMap::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let url = events_url(from, to, page_token.as_deref())
            .ok_or_else(|| BackendError::invalid_input("La ventana de sincronización no es válida."))?;
        let page = block_on(session.call(Method::GET, &url, None))?;
        for item in page.get("items").and_then(Value::as_array).into_iter().flatten() {
            if let Some(event) = google_event_in(item, &Local) {
                events.insert(event.id.clone(), event);
            }
        }
        page_token = page.get("nextPageToken").and_then(Value::as_str).map(str::to_string);
        if page_token.is_none() {
            break;
        }
    }
    Ok(events)
}

/// A linked event the window did not list: it moved away or was deleted.
fn read_single(session: &CalendarSession, google_id: &str) -> Result<GoogleEvent, BackendError> {
    match block_on(session.call(Method::GET, &event_url(google_id), None)) {
        Ok(value) => google_event_in(&value, &Local).ok_or_else(|| BackendError::invalid_input("Google devolvió un evento inválido.")),
        Err(error) if error.code == BackendErrorCode::NotFound => Ok(GoogleEvent {
            id: google_id.to_string(),
            ical_uid: None,
            updated: String::new(),
            cancelled: true,
            title: String::new(),
            timing: None,
        }),
        Err(error) => Err(error),
    }
}

fn load_links(connection: &Connection, owner: &str) -> AgendaResult<Vec<Link>> {
    let mut statement = connection.prepare(
        "SELECT account_email, google_event_id, event_id, ical_uid, google_updated, notia_fingerprint
         FROM agenda_google_links WHERE owner_user_id=?1",
    )?;
    let links = statement
        .query_map([owner], |row| {
            Ok(Link {
                account: row.get(0)?,
                google_id: row.get(1)?,
                event_id: row.get(2)?,
                ical_uid: row.get(3)?,
                google_updated: row.get(4)?,
                fingerprint: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(links)
}

/// Agenda events of the window, plus every linked one wherever it is.
fn load_events(connection: &Connection, owner: &str, from: NaiveDate, to: NaiveDate) -> AgendaResult<Vec<EventRecord>> {
    query_events(
        connection,
        "SELECT id, date, start_minute, end_minute, title, priority FROM agenda_events
         WHERE owner_user_id=?1 AND (date BETWEEN ?2 AND ?3
             OR id IN (SELECT event_id FROM agenda_google_links WHERE owner_user_id=?1))
         ORDER BY date, start_minute",
        params![owner, date_key(from), date_key(to)],
    )
}

fn apply_writes(connection: &Connection, owner: &str, writes: &[Write]) -> AgendaResult<bool> {
    let now = crate::finance::now();
    let mut agenda_changed = false;
    for write in writes {
        match write {
            Write::InsertEvent(event) => {
                connection.execute(
                    "INSERT INTO agenda_events
                     (id, owner_user_id, date, start_minute, end_minute, title, priority, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![event.id, owner, date_key(event.date), event.start_minute, event.end_minute, event.title, event.priority.as_str(), now],
                )?;
                agenda_changed = true;
            }
            Write::UpdateEvent(event) => {
                agenda_changed |= connection.execute(
                    "UPDATE agenda_events SET date=?1, start_minute=?2, end_minute=?3, title=?4, updated_at=?5
                     WHERE id=?6 AND owner_user_id=?7",
                    params![date_key(event.date), event.start_minute, event.end_minute, event.title, now, event.id, owner],
                )? > 0;
            }
            Write::DeleteEvent(id) => {
                agenda_changed |= connection.execute("DELETE FROM agenda_events WHERE id=?1 AND owner_user_id=?2", params![id, owner])? > 0;
            }
            Write::SaveLink(link) => {
                connection.execute(
                    "INSERT INTO agenda_google_links
                     (account_email, google_event_id, event_id, owner_user_id, ical_uid, google_updated, notia_fingerprint, synced_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(account_email, google_event_id) DO UPDATE SET
                         event_id=excluded.event_id, ical_uid=excluded.ical_uid, google_updated=excluded.google_updated,
                         notia_fingerprint=excluded.notia_fingerprint, synced_at=excluded.synced_at",
                    params![link.account, link.google_id, link.event_id, owner, link.ical_uid, link.google_updated, link.fingerprint, now],
                )?;
            }
            Write::DeleteLink { account, google_id } => {
                connection.execute(
                    "DELETE FROM agenda_google_links WHERE account_email=?1 AND google_event_id=?2 AND owner_user_id=?3",
                    params![account, google_id, owner],
                )?;
            }
        }
    }
    Ok(agenda_changed)
}

/// Google rejected this one event (not the account, not the connection):
/// the sync goes on with the others.
fn only_this_event(error: &BackendError) -> bool {
    !error.retryable && error.code != BackendErrorCode::Unauthorized
}

fn agenda_error(error: crate::agenda::AgendaCommandError) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, error.message, true)
}

/// The link of an event Google now has as `created` (the answer to a create or an update).
fn link_to(account: &str, event: &EventRecord, created: &Value) -> Option<Link> {
    let google_id = created.get("id").and_then(Value::as_str).filter(|id| valid_google_id(id))?;
    Some(Link {
        account: account.to_string(),
        google_id: google_id.to_string(),
        event_id: event.id.clone(),
        ical_uid: created.get("iCalUID").and_then(Value::as_str).map(str::to_string),
        google_updated: created.get("updated").and_then(Value::as_str).unwrap_or_default().to_string(),
        fingerprint: fingerprint(event),
    })
}

/// Syncs one account; `true` when the Agenda changed.
fn sync_account(app: &AppHandle, library_id: &str, context: &AgendaContext, account: &MailAccountRef, is_target: bool) -> Result<bool, BackendError> {
    let session = calendar_session(app, library_id, account)?;
    let today = Local::now().date_naive();
    let (from, to) = (today - Duration::days(PAST_DAYS), today + Duration::days(FUTURE_DAYS));
    let (events, links) = with_transaction(app, context, false, |connection, owner, _| {
        Ok((load_events(connection, owner, from, to)?, load_links(connection, owner)?))
    })
    .map_err(agenda_error)?;
    let window: Vec<String> = events.iter().filter(|event| event.date >= from && event.date <= to).map(|event| event.id.clone()).collect();
    let events: HashMap<String, EventRecord> = events.into_iter().map(|event| (event.id.clone(), event)).collect();

    let account_key = account.email.trim().to_lowercase();
    let mut google = list_window(&session, from, to)?;
    let unseen: Vec<&Link> = links
        .iter()
        .filter(|link| link.account == account_key && !google.contains_key(&link.google_id))
        // A linked event of the window that Google did not list, or one Notia deleted.
        .filter(|link| events.get(&link.event_id).is_none_or(|event| event.date >= from && event.date <= to))
        .take(MAX_SINGLE_READS)
        .collect();
    for link in unseen {
        match read_single(&session, &link.google_id) {
            Ok(remote) => {
                google.insert(link.google_id.clone(), remote);
            }
            Err(error) if only_this_event(&error) => log::error!("[notia:agenda-sync] un evento no se pudo leer: {:?}", error.code),
            Err(error) => return Err(error),
        }
    }

    let snapshot = Snapshot { events: &events, links: &links, window: &window };
    let actions = plan(&account_key, is_target, &snapshot, &google);
    let link_of = |google_id: &str| links.iter().find(|link| link.account == account_key && link.google_id == google_id);
    let mut writes = Vec::new();
    for action in actions {
        match run_action(&session, &account_key, action, &events, &google, link_of) {
            Ok(mut done) => writes.append(&mut done),
            Err(error) if only_this_event(&error) => {
                log::error!("[notia:agenda-sync] un evento no se sincronizó: {:?}", error.code)
            }
            Err(error) => {
                // Keeps what already happened in Google before giving up.
                save(app, context, &writes)?;
                return Err(error);
            }
        }
    }
    save(app, context, &writes)
}

fn save(app: &AppHandle, context: &AgendaContext, writes: &[Write]) -> Result<bool, BackendError> {
    if writes.is_empty() {
        return Ok(false);
    }
    with_transaction(app, context, true, |connection, owner, _| apply_writes(connection, owner, writes)).map_err(agenda_error)
}

fn run_action<'a>(
    session: &CalendarSession,
    account: &str,
    action: Action,
    events: &HashMap<String, EventRecord>,
    google: &HashMap<String, GoogleEvent>,
    link_of: impl Fn(&str) -> Option<&'a Link>,
) -> Result<Vec<Write>, BackendError> {
    let create = |event: &EventRecord| -> Result<Link, BackendError> {
        let body = google_body_in(event, &Local).ok_or_else(|| BackendError::invalid_input("La hora del evento no es válida."))?;
        let created = block_on(session.call(Method::POST, &format!("{CALENDAR_API}/events"), Some(body)))?;
        link_to(account, event, &created).ok_or_else(|| BackendError::invalid_input("Google no devolvió el evento creado."))
    };
    let forget = |google_id: &str| Write::DeleteLink { account: account.to_string(), google_id: google_id.to_string() };
    Ok(match action {
        Action::CreateInGoogle { event_id } => {
            let Some(event) = events.get(&event_id) else { return Ok(Vec::new()) };
            vec![Write::SaveLink(create(event)?)]
        }
        Action::UpdateGoogle { google_id, event_id } => {
            let Some(event) = events.get(&event_id) else { return Ok(Vec::new()) };
            let body = google_body_in(event, &Local).ok_or_else(|| BackendError::invalid_input("La hora del evento no es válida."))?;
            match block_on(session.call(Method::PATCH, &event_url(&google_id), Some(body))) {
                Ok(updated) => link_to(account, event, &updated).map(Write::SaveLink).into_iter().collect(),
                Err(error) if error.code == BackendErrorCode::NotFound => vec![forget(&google_id), Write::SaveLink(create(event)?)],
                // An event of someone else's calendar: Notia keeps its version and stops trying.
                Err(error) if only_this_event(&error) => {
                    let Some(link) = link_of(&google_id) else { return Err(error) };
                    vec![Write::SaveLink(Link { fingerprint: fingerprint(event), ..link.clone() })]
                }
                Err(error) => return Err(error),
            }
        }
        Action::DeleteInGoogle { google_id } => match block_on(session.call(Method::DELETE, &event_url(&google_id), None)) {
            Ok(_) => vec![forget(&google_id)],
            Err(error) if only_this_event(&error) => vec![forget(&google_id)],
            Err(error) => return Err(error),
        },
        Action::RecreateInGoogle { google_id, event_id } => {
            let Some(event) = events.get(&event_id) else { return Ok(vec![forget(&google_id)]) };
            vec![forget(&google_id), Write::SaveLink(create(event)?)]
        }
        Action::Import { google_id } => {
            let Some(remote) = google.get(&google_id) else { return Ok(Vec::new()) };
            let Some(timing) = remote.timing else { return Ok(Vec::new()) };
            let event = EventRecord {
                id: Uuid::new_v4().to_string(),
                date: timing.date,
                start_minute: timing.start_minute,
                end_minute: timing.end_minute,
                title: remote.title.clone(),
                priority: AgendaPriority::DEFAULT,
            };
            let link = Link {
                account: account.to_string(),
                google_id: remote.id.clone(),
                event_id: event.id.clone(),
                ical_uid: remote.ical_uid.clone(),
                google_updated: remote.updated.clone(),
                fingerprint: fingerprint(&event),
            };
            vec![Write::InsertEvent(event), Write::SaveLink(link)]
        }
        Action::Adopt { google_id, event_id } => {
            let (Some(remote), Some(event)) = (google.get(&google_id), events.get(&event_id)) else { return Ok(Vec::new()) };
            vec![Write::SaveLink(Link {
                account: account.to_string(),
                google_id: remote.id.clone(),
                event_id: event.id.clone(),
                ical_uid: remote.ical_uid.clone(),
                google_updated: remote.updated.clone(),
                fingerprint: fingerprint(event),
            })]
        }
        Action::UpdateNotia { google_id, event_id } => {
            let (Some(remote), Some(current), Some(link)) = (google.get(&google_id), events.get(&event_id), link_of(&google_id)) else {
                return Ok(Vec::new());
            };
            let Some(timing) = remote.timing else { return Ok(Vec::new()) };
            let event = EventRecord {
                date: timing.date,
                start_minute: timing.start_minute,
                end_minute: timing.end_minute,
                title: remote.title.clone(),
                ..current.clone()
            };
            let link = Link { google_updated: remote.updated.clone(), fingerprint: fingerprint(&event), ..link.clone() };
            vec![Write::UpdateEvent(event), Write::SaveLink(link)]
        }
        Action::DeleteInNotia { google_id, event_id } => vec![Write::DeleteEvent(event_id), forget(&google_id)],
        Action::ForgetLink { google_id } => vec![forget(&google_id)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn buenos_aires() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn event(id: &str, day: &str, start: u16, end: u16, title: &str) -> EventRecord {
        EventRecord { id: id.into(), date: date(day), start_minute: start, end_minute: end, title: title.into(), priority: AgendaPriority::Medium }
    }

    fn remote(id: &str, updated: &str, timing: Option<(&str, u16, u16)>) -> GoogleEvent {
        GoogleEvent {
            id: id.into(),
            ical_uid: Some(format!("{id}@google.com")),
            updated: updated.into(),
            cancelled: false,
            title: format!("Evento {id}"),
            timing: timing.map(|(day, start, end)| Timing { date: date(day), start_minute: start, end_minute: end }),
        }
    }

    fn link(account: &str, google_id: &str, event: &EventRecord, updated: &str) -> Link {
        Link {
            account: account.into(),
            google_id: google_id.into(),
            event_id: event.id.clone(),
            ical_uid: Some(format!("{google_id}@google.com")),
            google_updated: updated.into(),
            fingerprint: fingerprint(event),
        }
    }

    fn plan_of(is_target: bool, events: Vec<EventRecord>, links: Vec<Link>, google: Vec<GoogleEvent>) -> Vec<Action> {
        let window: Vec<String> = events.iter().map(|event| event.id.clone()).collect();
        let events: HashMap<String, EventRecord> = events.into_iter().map(|event| (event.id.clone(), event)).collect();
        let google: HashMap<String, GoogleEvent> = google.into_iter().map(|event| (event.id.clone(), event)).collect();
        plan("ana@gmail.com", is_target, &Snapshot { events: &events, links: &links, window: &window }, &google)
    }

    #[test]
    fn timed_events_land_on_the_grid_in_local_time() {
        let zone = buenos_aires();
        assert_eq!(
            timing_in("2026-10-21T21:10:00-03:00", "2026-10-21T22:50:00-03:00", &zone),
            Some(Timing { date: date("2026-10-21"), start_minute: 1260, end_minute: 1380 })
        );
        // Written in UTC, read in Buenos Aires.
        assert_eq!(
            timing_in("2026-10-22T00:00:00Z", "2026-10-22T01:00:00Z", &zone),
            Some(Timing { date: date("2026-10-21"), start_minute: 1260, end_minute: 1320 })
        );
        // Past midnight it ends at 24:00; a few minutes still take a block.
        assert_eq!(timing_in("2026-10-21T23:00:00-03:00", "2026-10-22T02:00:00-03:00", &zone).unwrap().end_minute, DAY_MINUTES);
        assert_eq!(timing_in("2026-10-21T09:00:00-03:00", "2026-10-21T09:05:00-03:00", &zone).unwrap().end_minute, 555);
        assert_eq!(timing_in("2026-10-21T09:00:00-03:00", "2026-10-21T09:00:00-03:00", &zone), None);
    }

    #[test]
    fn google_events_are_read_and_all_day_ones_have_no_blocks() {
        let zone = buenos_aires();
        let timed = json!({ "id": "abc123", "status": "confirmed", "updated": "2026-09-27T10:00:00.000Z", "iCalUID": "abc123@google.com",
            "summary": "  Iron\nMaiden  ", "start": { "dateTime": "2026-10-21T21:00:00-03:00" }, "end": { "dateTime": "2026-10-22T00:00:00-03:00" } });
        let read = google_event_in(&timed, &zone).unwrap();
        assert_eq!(read.title, "Iron Maiden");
        assert_eq!(read.timing, Some(Timing { date: date("2026-10-21"), start_minute: 1260, end_minute: DAY_MINUTES }));
        let all_day = json!({ "id": "day1", "status": "confirmed", "start": { "date": "2026-10-21" }, "end": { "date": "2026-10-22" } });
        assert_eq!(google_event_in(&all_day, &zone).unwrap().timing, None);
        let cancelled = json!({ "id": "gone_20261021T120000Z", "status": "cancelled" });
        assert!(google_event_in(&cancelled, &zone).unwrap().cancelled);
        assert!(google_event_in(&json!({ "id": "../calendars" }), &zone).is_none());
        assert_eq!(google_event_in(&json!({ "id": "x1", "summary": "" }), &zone).unwrap().title, UNTITLED_EVENT);
    }

    #[test]
    fn notia_events_go_to_google_with_their_offset() {
        let body = google_body_in(&event("e1", "2026-10-21", 1260, DAY_MINUTES, "Recital"), &buenos_aires()).unwrap();
        assert_eq!(
            body,
            json!({ "summary": "Recital", "start": { "dateTime": "2026-10-21T21:00:00-03:00" }, "end": { "dateTime": "2026-10-22T00:00:00-03:00" } })
        );
    }

    #[test]
    fn new_events_on_each_side_reach_the_other() {
        let notia = event("n1", "2026-10-01", 540, 600, "Dentista");
        let actions = plan_of(true, vec![notia], vec![], vec![remote("g1", "u1", Some(("2026-10-02", 600, 660)))]);
        assert_eq!(actions, vec![Action::Import { google_id: "g1".into() }, Action::CreateInGoogle { event_id: "n1".into() }]);
        // Only the target account receives the events made in Notia.
        let other = plan_of(false, vec![event("n1", "2026-10-01", 540, 600, "Dentista")], vec![], vec![]);
        assert!(other.is_empty());
    }

    #[test]
    fn a_change_on_one_side_follows_to_the_other() {
        let notia = event("n1", "2026-10-01", 540, 600, "Dentista");
        let synced = link("ana@gmail.com", "g1", &notia, "u1");
        let unchanged = plan_of(true, vec![notia.clone()], vec![synced.clone()], vec![remote("g1", "u1", Some(("2026-10-01", 540, 600)))]);
        assert!(unchanged.is_empty());

        let google_moved = plan_of(true, vec![notia.clone()], vec![synced.clone()], vec![remote("g1", "u2", Some(("2026-10-01", 600, 660)))]);
        assert_eq!(google_moved, vec![Action::UpdateNotia { google_id: "g1".into(), event_id: "n1".into() }]);

        let renamed = EventRecord { title: "Dentista (control)".into(), ..notia.clone() };
        let notia_changed = plan_of(true, vec![renamed.clone()], vec![synced.clone()], vec![remote("g1", "u1", Some(("2026-10-01", 540, 600)))]);
        assert_eq!(notia_changed, vec![Action::UpdateGoogle { google_id: "g1".into(), event_id: "n1".into() }]);

        // Both changed: Notia wins.
        let both = plan_of(true, vec![renamed], vec![synced], vec![remote("g1", "u2", Some(("2026-10-01", 600, 660)))]);
        assert_eq!(both, vec![Action::UpdateGoogle { google_id: "g1".into(), event_id: "n1".into() }]);
    }

    #[test]
    fn deletions_follow_unless_notia_changed_the_event() {
        let notia = event("n1", "2026-10-01", 540, 600, "Dentista");
        let synced = link("ana@gmail.com", "g1", &notia, "u1");
        let cancelled = GoogleEvent { cancelled: true, ..remote("g1", "u2", None) };

        let notia_deleted = plan_of(true, vec![], vec![synced.clone()], vec![remote("g1", "u2", Some(("2026-10-01", 540, 600)))]);
        assert_eq!(notia_deleted, vec![Action::DeleteInGoogle { google_id: "g1".into() }]);

        let google_deleted = plan_of(true, vec![notia.clone()], vec![synced.clone()], vec![cancelled.clone()]);
        assert_eq!(google_deleted, vec![Action::DeleteInNotia { google_id: "g1".into(), event_id: "n1".into() }]);

        let renamed = EventRecord { title: "Otro".into(), ..notia };
        let notia_wins = plan_of(true, vec![renamed], vec![synced.clone()], vec![cancelled.clone()]);
        assert_eq!(notia_wins, vec![Action::RecreateInGoogle { google_id: "g1".into(), event_id: "n1".into() }]);

        let both_deleted = plan_of(true, vec![], vec![synced], vec![cancelled]);
        assert_eq!(both_deleted, vec![Action::ForgetLink { google_id: "g1".into() }]);
    }

    #[test]
    fn an_event_made_on_both_sides_before_the_first_sync_becomes_one() {
        let notia = event("n1", "2026-10-21", 1260, DAY_MINUTES, "Evento g1");
        let actions = plan_of(true, vec![notia], vec![], vec![remote("g1", "u1", Some(("2026-10-21", 1260, DAY_MINUTES)))]);
        assert_eq!(actions, vec![Action::Adopt { google_id: "g1".into(), event_id: "n1".into() }]);
    }

    #[test]
    fn a_shared_meeting_comes_in_once_and_all_day_events_stay_out() {
        let notia = event("n1", "2026-10-01", 540, 600, "Reunión");
        let from_work = link("trabajo@empresa.com", "w1", &notia, "u1");
        let same_meeting = GoogleEvent { ical_uid: Some("w1@google.com".into()), ..remote("p1", "u1", Some(("2026-10-01", 540, 600))) };
        let all_day = remote("d1", "u1", None);
        let twice = GoogleEvent { ical_uid: Some("r@google.com".into()), ..remote("r1", "u1", Some(("2026-10-03", 540, 600))) };
        let twice_again = GoogleEvent { ical_uid: Some("r@google.com".into()), ..remote("r2", "u1", Some(("2026-10-03", 540, 600))) };
        let actions = plan_of(true, vec![notia], vec![from_work], vec![same_meeting, all_day, twice, twice_again]);
        assert_eq!(actions, vec![Action::Import { google_id: "r1".into() }]);
    }

    #[test]
    fn every_instance_of_a_recurring_event_comes_in() {
        let weekly = |id: &str, day: &str| GoogleEvent { ical_uid: Some("weekly@google.com".into()), ..remote(id, "u1", Some((day, 540, 600))) };
        let actions = plan_of(false, vec![], vec![], vec![weekly("w_20261001", "2026-10-01"), weekly("w_20261008", "2026-10-08")]);
        assert_eq!(actions, vec![Action::Import { google_id: "w_20261001".into() }, Action::Import { google_id: "w_20261008".into() }]);
    }

    #[test]
    fn links_of_other_accounts_are_left_alone() {
        let notia = event("n1", "2026-10-01", 540, 600, "Reunión");
        let other = link("trabajo@empresa.com", "w1", &notia, "u1");
        let actions = plan_of(true, vec![], vec![other], vec![]);
        assert!(actions.is_empty());
    }

    #[test]
    fn the_target_is_the_first_personal_account() {
        let work = MailAccountRef { email: "t@empresa.com".into(), account_type: MailAccountType::Laboral };
        let personal = MailAccountRef { email: "a@gmail.com".into(), account_type: MailAccountType::Personal };
        assert_eq!(target_account(&[work.clone(), personal.clone()]), Some(&personal));
        assert_eq!(target_account(std::slice::from_ref(&work)), Some(&work));
        assert_eq!(target_account(&[]), None);
    }

    #[test]
    fn the_window_is_asked_in_utc_with_deleted_events() {
        let url = events_url(date("2026-09-01"), date("2026-09-30"), Some("tok+en/=")).unwrap();
        assert!(url.starts_with(&format!("{CALENDAR_API}/events?singleEvents=true&showDeleted=true")));
        assert!(url.contains("timeMin=") && url.contains("Z&timeMax="));
        assert!(url.ends_with("&pageToken=tok%2Ben%2F%3D"));
    }

    #[test]
    fn writes_keep_links_and_events_together() {
        let connection = crate::agenda::tests::connection();
        let owner = OWNER;
        let new_event = event("n1", "2026-10-01", 540, 600, "Dentista");
        let synced = link("ana@gmail.com", "g1", &new_event, "u1");
        assert!(apply_writes(&connection, owner, &[Write::InsertEvent(new_event.clone()), Write::SaveLink(synced.clone())]).unwrap());
        let moved = EventRecord { start_minute: 600, end_minute: 660, ..new_event.clone() };
        let relinked = Link { google_updated: "u2".into(), fingerprint: fingerprint(&moved), ..synced };
        assert!(apply_writes(&connection, owner, &[Write::UpdateEvent(moved.clone()), Write::SaveLink(relinked.clone())]).unwrap());
        let events = load_events(&connection, owner, date("2026-01-01"), date("2026-01-02")).unwrap();
        assert_eq!(events, vec![moved]);
        assert_eq!(load_links(&connection, owner).unwrap(), vec![relinked]);
        let forget = Write::DeleteLink { account: "ana@gmail.com".into(), google_id: "g1".into() };
        assert!(apply_writes(&connection, owner, &[Write::DeleteEvent("n1".into()), forget]).unwrap());
        assert!(load_links(&connection, owner).unwrap().is_empty());
        assert!(!apply_writes(&connection, owner, &[Write::DeleteEvent("n1".into())]).unwrap());
    }
}
