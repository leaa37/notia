//! Waits of a task: a step that depends on something that has not happened
//! yet (a reply by mail, a date, someone doing something) leaves the task
//! waiting instead of failing. The executor calls `wait_ai_task`, ends its
//! run with what it did so far, and no thread is held: the adapter checks
//! the waiting tasks every minute and, when what was awaited happens or
//! its deadline passes, the executor runs the same step again with what
//! happened while it waited.
//!
//! - `mail`: a new message in a Gmail thread (a reply to a sent mail) or one
//!   that matches a search; the adapter looks every `MAIL_POLL_MS` without
//!   calling the model.
//! - `whatsapp`: a reply in a WhatsApp chat (a company's bot or support),
//!   once its author stopped writing for `reply_wait::WHATSAPP_TASK_WAIT`;
//!   the adapter looks when a message arrives in the chat, and at least
//!   every `WHATSAPP_POLL_MS`, without calling the model.
//! - `time`: until a moment.
//! - `check`: something only the agent can tell (a ticket moved, a file
//!   appeared): the executor runs every `checkEveryMinutes` to look and
//!   waits again when it did not happen yet.

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{clipped, AiTask, AiTaskStatus, StepStatus, MAX_REPORT_CHARS};
use crate::ai_actions::labels::moment_label;
use crate::ai_actions::schedule::local_date;
use crate::error::BackendError;

pub const WAIT_TOOL: &str = "wait_ai_task";
/// How often the adapter looks at the mail a task waits for.
pub const MAIL_POLL_MS: i64 = 5 * 60 * 1000;
/// How often it looks at the WhatsApp chat at least (the store, nothing
/// goes out); a message that arrives there makes it look sooner.
pub const WHATSAPP_POLL_MS: i64 = 60 * 1000;
pub const MIN_CHECK_MINUTES: i64 = 10;
pub const DEFAULT_CHECK_MINUTES: i64 = 60;
pub const DEFAULT_WAIT_HOURS: f64 = 72.0;
/// The longest a wait may last: 30 days.
pub const MAX_WAIT_HOURS: f64 = 720.0;
pub const MAX_REASON_CHARS: usize = 300;
const MAX_QUERY_CHARS: usize = 300;
/// New messages a wake lists.
pub const MAX_WAKE_MESSAGES: usize = 10;
/// Characters of the step's notes the executor reads again.
const NOTES_CHARS_IN_REQUEST: usize = 6_000;
const MINUTE_MS: i64 = 60 * 1000;
const HOUR_MS: f64 = 60.0 * 60.0 * 1000.0;

/// What a step waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WaitFor {
    /// A new message in `thread_id`, or one that matches `query`, that
    /// arrived after the wait started and was not sent by the account.
    #[serde(rename_all = "camelCase")]
    Mail {
        account: String,
        #[serde(default)]
        thread_id: Option<String>,
        #[serde(default)]
        query: Option<String>,
    },
    /// What others write in `chat` from `since_ms` on (the message the task
    /// sent). `name` is the chat's name when the wait began, for the panel.
    #[serde(rename = "whatsapp", rename_all = "camelCase")]
    WhatsApp { chat: String, name: String, since_ms: i64 },
    Time,
    #[serde(rename_all = "camelCase")]
    Check { every_ms: i64 },
}

/// How often the adapter looks at what a wait waits for, when it looks.
pub fn poll_ms(wait_for: &WaitFor) -> i64 {
    match wait_for {
        WaitFor::WhatsApp { .. } => WHATSAPP_POLL_MS,
        _ => MAIL_POLL_MS,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskWait {
    pub step_id: String,
    /// What it waits for, for the person: «Respuesta de soporte sobre el acceso».
    pub reason: String,
    #[serde(rename = "for")]
    pub wait_for: WaitFor,
    pub started_at_ms: i64,
    /// When to look next: the moment of a `time` wait, the next check, the
    /// next look at the mail.
    pub wake_at_ms: i64,
    /// When it stops waiting without what it waited for.
    pub deadline_ms: i64,
    /// Why the last look at the mail failed, if it did.
    #[serde(default)]
    pub last_error: Option<String>,
}

fn text(arguments: &Value, name: &str) -> Option<String> {
    arguments.get(name).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn number(arguments: &Value, name: &str) -> Option<f64> {
    match arguments.get(name)? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

/// A moment written by the model: with an offset or `Z`, or a local date
/// and time (`2026-11-03T10:00`, `2026-11-03 10:00`) in `zone`.
pub fn parse_moment(value: &str, zone: &TimeZone) -> Option<i64> {
    let value = value.trim();
    if let Ok(timestamp) = value.parse::<Timestamp>() {
        return Some(timestamp.as_millisecond());
    }
    let local = value.replacen(' ', "T", 1).parse::<DateTime>().ok()?;
    zone.to_ambiguous_zoned(local).compatible().ok().map(|zoned| zoned.timestamp().as_millisecond())
}

/// The wait a `wait_ai_task` call asks for, from `now_ms`. `accounts` are
/// the connected Gmail accounts: a `mail` wait names one of them, or the
/// only one there is.
pub fn wait_request(arguments: &Value, step_id: &str, accounts: &[String], zone: &TimeZone, now_ms: i64) -> Result<AiTaskWait, BackendError> {
    let reason = text(arguments, "reason").ok_or_else(|| BackendError::invalid_input("Falta reason: qué esperás, en una frase para la persona."))?;
    let kind = text(arguments, "kind").ok_or_else(|| BackendError::invalid_input("Falta kind: mail, whatsapp, time o check."))?;
    let hours = number(arguments, "maxWaitHours").unwrap_or(DEFAULT_WAIT_HOURS);
    if !(hours > 0.0 && hours <= MAX_WAIT_HOURS) {
        return Err(BackendError::invalid_input(format!("maxWaitHours va de más de 0 a {MAX_WAIT_HOURS} (30 días).")));
    }
    let mut deadline_ms = now_ms + (hours * HOUR_MS) as i64;
    let (wait_for, wake_at_ms) = match kind.as_str() {
        "mail" => {
            let account = mail_account(text(arguments, "account"), accounts)?;
            let thread_id = text(arguments, "threadId");
            let query = text(arguments, "query");
            if thread_id.is_none() && query.is_none() {
                return Err(BackendError::invalid_input(
                    "Un wait de mail necesita threadId (el hilo del correo que mandaste o del que esperás respuesta; send_gmail_message y list_gmail_messages lo devuelven) o query (una búsqueda de Gmail, por ejemplo «from:soporte@empresa.com»).",
                ));
            }
            if query.as_ref().is_some_and(|query| query.chars().count() > MAX_QUERY_CHARS) {
                return Err(BackendError::invalid_input(format!("query tiene más de {MAX_QUERY_CHARS} caracteres.")));
            }
            (WaitFor::Mail { account, thread_id, query }, now_ms + MAIL_POLL_MS)
        }
        // The adapter resolves the chat (and its name and where the replies
        // start) against the linked WhatsApp before the wait is saved.
        "whatsapp" => {
            let chat = text(arguments, "chat").ok_or_else(|| {
                BackendError::invalid_input("Un wait de whatsapp necesita chat: el id del chat (de whatsapp_list_chats o el que devolvió whatsapp_send_message) o su nombre.")
            })?;
            (WaitFor::WhatsApp { chat, name: String::new(), since_ms: now_ms }, now_ms + WHATSAPP_POLL_MS)
        }
        "time" => {
            let at = match (text(arguments, "until"), number(arguments, "afterMinutes")) {
                (Some(until), _) => parse_moment(&until, zone).ok_or_else(|| {
                    BackendError::invalid_input("until no es una fecha y hora válida: usá «2026-11-03T10:00» (hora local) o con zona.")
                })?,
                (None, Some(minutes)) if minutes > 0.0 => now_ms + (minutes * MINUTE_MS as f64) as i64,
                _ => return Err(BackendError::invalid_input("Un wait de time necesita until (fecha y hora) o afterMinutes.")),
            };
            if at <= now_ms {
                return Err(BackendError::invalid_input("until ya pasó: si no hay que esperar, seguí con el paso."));
            }
            if at > now_ms + (MAX_WAIT_HOURS * HOUR_MS) as i64 {
                return Err(BackendError::invalid_input("No se puede esperar más de 30 días."));
            }
            deadline_ms = at;
            (WaitFor::Time, at)
        }
        "check" => {
            let minutes = number(arguments, "checkEveryMinutes").map(|minutes| minutes.round() as i64).unwrap_or(DEFAULT_CHECK_MINUTES);
            if minutes < MIN_CHECK_MINUTES {
                return Err(BackendError::invalid_input(format!("checkEveryMinutes es de al menos {MIN_CHECK_MINUTES}: cada control es una corrida del Ejecutor.")));
            }
            let every_ms = minutes * MINUTE_MS;
            (WaitFor::Check { every_ms }, now_ms + every_ms)
        }
        other => return Err(BackendError::invalid_input(format!("kind «{other}» no existe: usá mail, whatsapp, time o check."))),
    };
    Ok(AiTaskWait {
        step_id: step_id.to_string(),
        reason: clipped(&reason, MAX_REASON_CHARS),
        wait_for,
        started_at_ms: now_ms,
        wake_at_ms: wake_at_ms.min(deadline_ms),
        deadline_ms,
        last_error: None,
    })
}

fn mail_account(asked: Option<String>, accounts: &[String]) -> Result<String, BackendError> {
    match asked {
        Some(asked) => accounts.iter().find(|account| account.eq_ignore_ascii_case(asked.trim())).cloned().ok_or_else(|| {
            BackendError::invalid_input(format!("La cuenta «{asked}» no está conectada; las conectadas son: {}.", accounts_list(accounts)))
        }),
        None => match accounts {
            [only] => Ok(only.clone()),
            [] => Err(BackendError::invalid_input("No hay cuentas de Gmail conectadas en la biblioteca: no se puede esperar un correo.")),
            _ => Err(BackendError::invalid_input(format!("Falta account: la cuenta que recibe la respuesta ({}).", accounts_list(accounts)))),
        },
    }
}

fn accounts_list(accounts: &[String]) -> String {
    if accounts.is_empty() {
        "ninguna".to_string()
    } else {
        accounts.join(", ")
    }
}

/// What the watch does with a waiting task now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    NotYet,
    /// Look at the mail; with nothing new and the deadline passed, wake.
    LookAtMail,
    /// Look at the WhatsApp chat, the same way.
    LookAtWhatsApp,
    /// Wake the executor with this note.
    Wake(String),
}

pub fn due(wait: &AiTaskWait, zone: &TimeZone, now_ms: i64) -> Due {
    let whatsapp = matches!(wait.wait_for, WaitFor::WhatsApp { .. });
    if !whatsapp && now_ms < wait.wake_at_ms && now_ms < wait.deadline_ms {
        return Due::NotYet;
    }
    match wait.wait_for {
        WaitFor::Mail { .. } => Due::LookAtMail,
        // Reading the store is cheap: the adapter looks whenever it is asked.
        WaitFor::WhatsApp { .. } => Due::LookAtWhatsApp,
        WaitFor::Time => Due::Wake(format!("Llegó el momento que esperabas ({}).", when(zone, wait.deadline_ms, now_ms))),
        WaitFor::Check { .. } if now_ms >= wait.deadline_ms => Due::Wake(deadline_note(wait, zone, now_ms)),
        WaitFor::Check { .. } => Due::Wake(format!(
            "Control de la espera: fijate con tus herramientas si ya pasó. Si pasó, seguí con el paso; si no, volvé a llamar wait_ai_task (kind check, con maxWaitHours hasta el plazo que quede) y terminá tu respuesta. El plazo vence {}.",
            when(zone, wait.deadline_ms, now_ms).to_lowercase()
        )),
    }
}

/// The note of a wait whose deadline passed without what it waited for.
pub fn deadline_note(wait: &AiTaskWait, zone: &TimeZone, now_ms: i64) -> String {
    let mut note = format!(
        "Se cumplió el plazo de la espera ({}) sin que pase lo que esperabas. Decidí: insistir (por ejemplo, otro correo, otro mensaje o un recordatorio) y volver a esperar con wait_ai_task, o terminar el paso explicando qué falta.",
        when(zone, wait.deadline_ms, now_ms).to_lowercase()
    );
    if let Some(error) = &wait.last_error {
        note.push_str(&format!(" La última revisión falló: {error}"));
    }
    note
}

/// The messages of a Gmail thread or of a search that count as new: they
/// arrived after `since_ms` and the account did not send them (nor are they
/// drafts, spam or trash).
pub fn new_messages(messages: &[Value], since_ms: i64) -> Vec<Value> {
    const SKIPPED: [&str; 4] = ["SENT", "DRAFT", "SPAM", "TRASH"];
    messages
        .iter()
        .filter(|message| {
            let arrived = message.get("internalDate").and_then(|value| value.as_str().and_then(|text| text.parse::<i64>().ok()).or_else(|| value.as_i64()));
            let labels = message.get("labelIds").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            arrived.is_some_and(|arrived| arrived > since_ms) && !labels.iter().filter_map(Value::as_str).any(|label| SKIPPED.contains(&label))
        })
        .take(MAX_WAKE_MESSAGES)
        .cloned()
        .collect()
}

/// The Gmail search of a `query` wait: only what arrived since it started.
pub fn mail_search(query: &str, since_ms: i64) -> String {
    format!("{} after:{}", query.trim(), since_ms.div_euclid(1000).saturating_sub(60))
}

/// The note of the new messages, from their summaries (`message_summary`).
pub fn mail_note(account: &str, summaries: &[Value]) -> String {
    let lines = summaries
        .iter()
        .map(|summary| {
            let field = |name: &str| summary.get(name).and_then(Value::as_str).unwrap_or_default().trim().to_string();
            format!("- id {} · hilo {} · de {} · «{}» · {}: {}", field("id"), field("threadId"), field("from"), field("subject"), field("date"), clipped(&field("snippet"), 300))
        })
        .collect::<Vec<_>>();
    format!(
        "Llegaron {} correo(s) nuevo(s) a {account}:\n{}\nLeelos completos con read_gmail_message (account {account}) antes de seguir.",
        summaries.len(),
        lines.join("\n")
    )
}

fn when(zone: &TimeZone, ms: i64, now_ms: i64) -> String {
    moment_label(zone, ms, local_date(zone, now_ms))
}

/// Leaves the task waiting after the executor's run of its step `index`:
/// what it did so far goes to the step's notes.
pub fn begin_wait(task: &mut AiTask, index: usize, report: &str) {
    let step = &mut task.steps[index];
    if !report.trim().is_empty() {
        step.notes.push(clipped(report, MAX_REPORT_CHARS));
    }
    step.status = StepStatus::Waiting;
    task.status = AiTaskStatus::Waiting;
    task.phase = None;
    task.pending = None;
}

/// Ends the wait: the executor runs its step again with `note`.
pub fn wake(task: &mut AiTask, note: &str) {
    let Some(wait) = task.wait.take() else { return };
    let note = format!("Esperabas: {}\n{}", wait.reason, note.trim());
    if let Some(step) = task.steps.iter_mut().find(|step| step.id == wait.step_id) {
        step.status = StepStatus::Pending;
        step.notes.push(format!("— Fin de la espera: {}", clipped(&note, 1_500)));
    }
    task.wake = Some(note);
    task.status = AiTaskStatus::Running;
    task.error = None;
}

/// What the executor reads again of the step it goes on with.
pub fn notes_for_request(notes: &[String]) -> String {
    let joined = notes.join("\n\n");
    let count = joined.chars().count();
    if count <= NOTES_CHARS_IN_REQUEST {
        return joined;
    }
    format!("…{}", joined.chars().skip(count - NOTES_CHARS_IN_REQUEST).collect::<String>())
}

/// «Respuesta por correo en el hilo (ana@x.com)», «Hasta Mañana · 10:00»…
pub fn kind_label(wait: &AiTaskWait, zone: &TimeZone, now_ms: i64) -> String {
    match &wait.wait_for {
        WaitFor::Mail { account, thread_id: Some(_), .. } => format!("Respuesta por correo en el hilo ({account})"),
        WaitFor::Mail { account, query, .. } => format!("Correo que coincida con «{}» ({account})", query.as_deref().unwrap_or_default()),
        WaitFor::WhatsApp { name, .. } => format!("Respuesta por WhatsApp de {name}"),
        WaitFor::Time => format!("Hasta {}", when(zone, wait.deadline_ms, now_ms).to_lowercase()),
        WaitFor::Check { every_ms } => format!("Controla {}", every_label(*every_ms)),
    }
}

fn every_label(every_ms: i64) -> String {
    let minutes = every_ms / MINUTE_MS;
    match minutes {
        60 => "cada hora".to_string(),
        minutes if minutes % 60 == 0 => format!("cada {} horas", minutes / 60),
        minutes => format!("cada {minutes} min"),
    }
}

/// When it looks next, for the panel; none for a `time` wait.
pub fn next_label(wait: &AiTaskWait, zone: &TimeZone, now_ms: i64) -> Option<String> {
    let next = when(zone, wait.wake_at_ms.max(now_ms), now_ms);
    match wait.wait_for {
        WaitFor::Mail { .. } => Some(format!("Próxima revisión del correo: {}", next.to_lowercase())),
        WaitFor::WhatsApp { .. } => Some(format!(
            "Sigue {} s después del último mensaje que llegue",
            crate::reply_wait::WHATSAPP_TASK_WAIT.as_secs()
        )),
        WaitFor::Check { .. } => Some(format!("Próximo control: {}", next.to_lowercase())),
        WaitFor::Time => None,
    }
}

pub fn deadline_label(wait: &AiTaskWait, zone: &TimeZone, now_ms: i64) -> String {
    match wait.wait_for {
        WaitFor::Time => format!("Sigue {}", when(zone, wait.deadline_ms, now_ms).to_lowercase()),
        _ => format!("Espera hasta {}", when(zone, wait.deadline_ms, now_ms).to_lowercase()),
    }
}

/// The note of a wait the person ended from the panel or the agent.
pub const RESUMED_NOTE: &str = "La persona pidió seguir ahora, sin esperar más. Fijate qué pasó y seguí con el paso.";
