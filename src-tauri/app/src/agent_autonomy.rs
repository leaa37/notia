//! The autonomous agent's clock. Every minute, for the selected library,
//! `thoughts.md`, `biography.md`, `talk.md`, `people.md` and `improve.md`
//! are created again when they are missing (a new `people.md` is filled with the people
//! the memory and the biography name), and once a day the agent files are reviewed (format
//! repaired, duplicates merged, sizes kept; see
//! `agent_knowledge::review_agent_files`). When the library's Telegram bot
//! runs on this device with the autonomous agent on, the Gmail accounts
//! are checked every two minutes for new mail in Recibidos, all of it in
//! one run. The hourly review is an AI action now (`ai_actions`).
//!
//! The runs go through the Telegram worker (`enqueue_autonomous`), so they
//! only happen where the bot runs and only reach the Owner. Each account's
//! Gmail history cursor is kept per device in `app_data/agent-autonomy/`,
//! so a restart does not announce old mail. The rules live in `backend_core::agent_autonomy`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::backend::agent_autonomy::{
    mail_trigger, new_mail, AutonomousKind, MAIL_POLL_INTERVAL_MS, MAX_TRIGGER_MAILS,
};
use crate::backend::mail_tools::{history_url, message_metadata_url, message_summary, parse_history_page, profile_history_id, profile_url, MailAccountRef};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Manager};
use crate::mail_tools::{gmail_accounts, gmail_session, GmailSession};
use crate::telegram_worker::{enqueue_autonomous, AutonomousEnqueue};

const TICK: Duration = Duration::from_secs(60);
/// The first tick waits for the app to open its library.
const FIRST_TICK_DELAY: Duration = Duration::from_secs(60);
/// History pages read per account and poll; more new mail than that in two
/// minutes is left for the hourly review.
const MAX_HISTORY_PAGES: usize = 5;
const STATE_DIRECTORY: &str = "agent-autonomy";
/// Time between two reviews of the agent files of a library.
const KNOWLEDGE_REVIEW_INTERVAL_MS: u64 = 24 * 60 * 60 * 1_000;

pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("agent-autonomy")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-agent-autonomy".into()).spawn(move || {
                std::thread::sleep(FIRST_TICK_DELAY);
                let mut clock = Clock::default();
                loop {
                    tick(&app, &mut clock);
                    std::thread::sleep(TICK);
                }
            });
            Ok(())
        })
        .build()
}

/// What the clock remembers between ticks, in memory only.
#[derive(Default)]
struct Clock {
    /// Whether the last check of the agent list files failed, so a lasting
    /// failure is logged once.
    files_failing: bool,
    last_mail_poll: Option<Instant>,
}

/// Per-device state of a library's autonomous agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutonomyState {
    /// Gmail history id reached per account address.
    #[serde(default)]
    gmail: HashMap<String, String>,
    /// When the agent files were last reviewed (ms since the epoch).
    #[serde(default)]
    knowledge_review_ms: Option<u64>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
}

/// Whether the agent files are due for a review: never reviewed on this
/// device, a day passed, or the clock went back.
fn review_due(last_ms: Option<u64>, now_ms: u64) -> bool {
    last_ms.is_none_or(|last| now_ms < last || now_ms - last >= KNOWLEDGE_REVIEW_INTERVAL_MS)
}

fn tick(app: &AppHandle, clock: &mut Clock) {
    // Only the computer that runs the library's services keeps the agent.
    if !crate::device_role::runs_services(app) {
        return;
    }
    let Some(library) = crate::library_catalog::selected_library(app) else {
        return;
    };
    if app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library.id).is_err() {
        return;
    }
    match crate::agent_workspace::ensure_agent_list_files(app, &library.id) {
        Ok(written) => {
            clock.files_failing = false;
            // A new `people.md` starts with the people the memory and the
            // biography already name.
            if written.contains(&crate::backend::agent_workspace::PEOPLE_PATH) {
                crate::agent_knowledge::schedule_organization(app, &library.id);
            }
        }
        Err(error) => {
            if !clock.files_failing {
                log::error!("[notia:autonomy] no se pudieron recrear los archivos del agente: {:?}", error.code);
            }
            clock.files_failing = true;
        }
    }
    let before = read_state(app, &library.id);
    let mut state = before.clone();
    // The review keeps the agent files usable whether or not Telegram runs
    // here; its organizations run in the background.
    let now = now_ms();
    if review_due(state.knowledge_review_ms, now) {
        state.knowledge_review_ms = Some(now);
        crate::agent_knowledge::review_agent_files(app, &library.id);
    }
    let enabled = crate::library_config::read_library_config(app, &library.id)
        .ok()
        .flatten()
        .is_some_and(|config| crate::backend::library_config::autonomous_agent_enabled(&config));
    if enabled
        && crate::telegram_worker::bot_runs_for(app, &library.id)
        && clock.last_mail_poll.is_none_or(|last| last.elapsed() >= Duration::from_millis(MAIL_POLL_INTERVAL_MS as u64))
    {
        clock.last_mail_poll = Some(Instant::now());
        watch_mail(app, &library.id, &mut state);
    }
    if state != before {
        write_state(app, &library.id, &state);
    }
}

/// Looks for new mail in every connected account and queues one run for
/// all of it. The cursors move on when the run was queued or nobody can
/// be told (the Owner has not linked Telegram); while another autonomous
/// run is busy they stay, so the next poll includes the same mail.
fn watch_mail(app: &AppHandle, library_id: &str, state: &mut AutonomyState) {
    let accounts = gmail_accounts(app, library_id);
    state.gmail.retain(|email, _| accounts.iter().any(|account| &account.email == email));
    let mut cursors = state.gmail.clone();
    let mut arrived = Vec::<(MailAccountRef, String)>::new();
    for (index, account) in accounts.iter().enumerate() {
        match poll_account(app, library_id, account, state.gmail.get(&account.email).map(String::as_str)) {
            Ok((cursor, added)) => {
                cursors.insert(account.email.clone(), cursor);
                arrived.extend(added.into_iter().map(|id| (account.clone(), id)));
            }
            // Neither the address nor the error body: only which account.
            Err(error) => log::error!(
                "[notia:autonomy] la cuenta {} ({}) no se revisó: {:?}",
                index + 1,
                account.account_type.id(),
                error.code
            ),
        }
    }
    if arrived.is_empty() {
        state.gmail = cursors;
        return;
    }
    let mails = arrived
        .iter()
        .take(MAX_TRIGGER_MAILS)
        .filter_map(|(account, id)| read_mail(app, library_id, account, id))
        .collect::<Vec<_>>();
    let trigger = mail_trigger(&crate::local_time::local_now().1, &mails, arrived.len());
    if enqueue_autonomous(app, library_id, AutonomousKind::NewMail, trigger) != AutonomousEnqueue::Busy {
        state.gmail = cursors;
    }
}

/// The account's new cursor and the messages that arrived since `cursor`.
/// Without a cursor (first look) or with one Gmail no longer knows, it
/// starts from now: mail that already arrived does not count.
fn poll_account(
    app: &AppHandle,
    library_id: &str,
    account: &MailAccountRef,
    cursor: Option<&str>,
) -> Result<(String, Vec<String>), BackendError> {
    // Getting the session may refresh the token, which blocks by itself.
    let session = gmail_session(app, library_id, account)?;
    crate::host::async_runtime::block_on(async {
        let Some(start) = cursor else {
            return Ok((current_cursor(&session).await?, Vec::new()));
        };
        let mut added = Vec::<String>::new();
        let mut latest = start.to_string();
        let mut page_token = None::<String>;
        for _ in 0..MAX_HISTORY_PAGES {
            let answer = match session.get(&history_url(start, page_token.as_deref())).await {
                Ok(answer) => answer,
                Err(error) if error.code == BackendErrorCode::NotFound => return Ok((current_cursor(&session).await?, Vec::new())),
                Err(error) => return Err(error),
            };
            let page = parse_history_page(&answer);
            for id in page.added {
                if !added.contains(&id) {
                    added.push(id);
                }
            }
            if let Some(id) = page.history_id {
                latest = id;
            }
            match page.next_page_token {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }
        Ok((latest, added))
    })
}

async fn current_cursor(session: &GmailSession) -> Result<String, BackendError> {
    profile_history_id(&session.get(&profile_url()).await?)
        .ok_or_else(|| BackendError::new(BackendErrorCode::ProviderUnavailable, "Gmail no devolvió el historial de la cuenta.", true))
}

/// Sender, subject, date and snippet of one new message, or `None` when it
/// cannot be read (for example, it was deleted meanwhile).
fn read_mail(app: &AppHandle, library_id: &str, account: &MailAccountRef, id: &str) -> Option<crate::backend::agent_autonomy::NewMail> {
    let session = gmail_session(app, library_id, account).ok()?;
    let message = crate::host::async_runtime::block_on(session.get(&message_metadata_url(id))).ok()?;
    Some(new_mail(&message_summary(&message, &[]), account))
}

fn state_file(app: &AppHandle, library_id: &str) -> Option<PathBuf> {
    let key = library_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect::<String>();
    app.path().app_data_dir().ok().map(|directory| directory.join(STATE_DIRECTORY).join(format!("{key}.json")))
}

fn read_state(app: &AppHandle, library_id: &str) -> AutonomyState {
    state_file(app, library_id)
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_state(app: &AppHandle, library_id: &str, state: &AutonomyState) {
    let Some(file) = state_file(app, library_id) else {
        return;
    };
    let Ok(text) = serde_json::to_string(state) else {
        return;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = file.with_extension("json.tmp");
    if std::fs::write(&temporary, text).is_err() || std::fs::rename(&temporary, &file).is_err() {
        log::error!("[notia:autonomy] no se pudo guardar el estado del agente autónomo");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_survives_a_round_trip_and_old_files() {
        let mut state = AutonomyState::default();
        state.gmail.insert("ana@gmail.com".into(), "901".into());
        let text = serde_json::to_string(&state).expect("json");
        assert_eq!(serde_json::from_str::<AutonomyState>(&text).expect("state"), state);
        assert_eq!(serde_json::from_str::<AutonomyState>("{}").expect("empty"), AutonomyState::default());
        // Files from before the hourly review became an AI action.
        let old = serde_json::from_str::<AutonomyState>(r#"{"lastReviewMs":42,"gmail":{"ana@gmail.com":"901"}}"#).expect("old");
        assert_eq!(old, state);
        state.knowledge_review_ms = Some(7);
        let text = serde_json::to_string(&state).expect("json");
        assert!(text.contains("\"knowledgeReviewMs\":7"));
        assert_eq!(serde_json::from_str::<AutonomyState>(&text).expect("state"), state);
    }

    #[test]
    fn the_agent_files_are_reviewed_once_a_day() {
        let day = KNOWLEDGE_REVIEW_INTERVAL_MS;
        assert!(review_due(None, 1_000));
        assert!(!review_due(Some(1_000), 1_000 + day - 1));
        assert!(review_due(Some(1_000), 1_000 + day));
        // A clock set back does not postpone the review for days.
        assert!(review_due(Some(10 * day), day));
    }
}
