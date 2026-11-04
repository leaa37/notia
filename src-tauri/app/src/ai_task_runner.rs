//! AI tasks of a library (the rules are in `backend_core::ai_tasks`).
//!
//! Only the agent creates a task, with `create_ai_task`, from a chat of the
//! app or Telegram. The task lives in the library's SQLite database for a
//! week and a thread of this process works it: the planner, one executor
//! run per step and the review, each a request of the backend runtime with
//! its prompt file (`execute_backend_request`, as Telegram drives a turn).
//! A confirmation, plan or question a run asks for waits for the Owner: in
//! the «Tareas IA» panel and, for a task created on Telegram, with buttons
//! in the Owner's chat. With «Confirmar todos» chosen when the task was
//! created, changes are applied without asking.
//!
//! A step that waits for something (`wait_ai_task`: a reply by mail, a
//! moment, a periodic check) ends the runner's thread and leaves the task
//! waiting in the database; the watch looks at the waiting tasks every
//! minute and starts the runner again when the wait ends. A waiting task
//! keeps waiting across restarts.
//!
//! Every change emits `TASK_CHANGED_EVENT` with the library id, and the
//! panel reloads. A task that was working when Notia closed is marked
//! interrupted at the next start and can be resumed.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::ai_actions::time_zone;
use crate::backend::ai_tasks::prompts::{
    approval_message, executor_request, finished_message, parse_plan, parse_review, planner_request, review_request, step_unfinished,
    step_without_report,
};
use crate::backend::ai_tasks::tools::{self, NewTask};
use crate::backend::ai_tasks::views::{self, AiTaskCard, AiTaskDetail, AiTaskThinking};
use crate::backend::ai_tasks::waits::{self, AiTaskWait, Due, WaitFor};
use crate::backend::ai_tasks::{
    clipped, clipped_report, AiTask, AiTaskApproval, AiTaskOrigin, AiTaskPhase, AiTaskStatus, AiTaskTurn, ApprovalKind, NextWork,
    ReviewEffect, StepStatus, RETENTION_MS,
};
use crate::backend::{
    AgentRequest, BackendActor, BackendChannel, BackendError, BackendErrorCode, BackendMessage, BackendRequest, BackendRequestContext,
    BackendRequestEnvelope, BackendResponse, BackendScope, MessageRole, OperationToken, PendingInteraction, PersistencePolicy,
    ProtocolVersion, ResumeDecision, ResumeRequest,
};
use crate::host::{AppHandle, Emitter, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};
use crate::library_users::LibraryDatabaseContext;

pub(crate) const TASK_CHANGED_EVENT: &str = "notia://ai-task-changed";
/// Prefix of the Telegram buttons of a task's approval.
pub(crate) const TELEGRAM_CALLBACK_PREFIX: &str = "aitask:";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// An approval nobody answers stops the task after a day.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const APPROVAL_POLL: Duration = Duration::from_secs(1);
/// The first clean-up waits for the app to open its library.
const FIRST_CLEANUP_DELAY: Duration = Duration::from_secs(30);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// How often the watch looks at the waiting tasks.
const WAIT_WATCH_INTERVAL: Duration = Duration::from_secs(60);
/// A look asked for a moment ends this long after it, so a reply counted
/// as quiet at that moment already is.
const LOOK_MARGIN: Duration = Duration::from_secs(1);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as i64).unwrap_or_default()
}

fn storage(error: impl std::fmt::Display) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, format!("No se pudo acceder a las Tareas IA: {error}"), true)
}

fn not_found() -> BackendError {
    BackendError::new(BackendErrorCode::NotFound, "La tarea ya no existe.", false)
}

fn owner_only() -> BackendError {
    BackendError::new(BackendErrorCode::Forbidden, "Las Tareas IA son solo del Owner de la biblioteca.", false)
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// The library database of `library_id`, from its binding.
fn database(app: &AppHandle, library_id: &str) -> Result<LibraryDatabaseContext, BackendError> {
    let binding = app.state::<LibraryBindingRegistry>().lookup(library_id)?;
    match binding.root {
        Some(LibraryBindingRoot::Desktop { canonical_root }) => {
            Ok(LibraryDatabaseContext { library_path: canonical_root.to_string_lossy().to_string(), android_directory_uri: None })
        }
        Some(LibraryBindingRoot::Android { tree_uri }) => {
            Ok(LibraryDatabaseContext { library_path: String::new(), android_directory_uri: Some(tree_uri.as_str().to_string()) })
        }
        None => Err(BackendError::new(BackendErrorCode::NotFound, "La biblioteca no está disponible.", true)),
    }
}

fn open(app: &AppHandle, database: &LibraryDatabaseContext) -> Result<Connection, BackendError> {
    let connection =
        crate::database::open_user_data_connection(app, &database.library_path, database.android_directory_uri.as_deref()).map_err(storage)?;
    connection.busy_timeout(BUSY_TIMEOUT).map_err(storage)?;
    Ok(connection)
}

/// Runs `work` and saves it (on Android, back through SAF).
fn write<T>(app: &AppHandle, database: &LibraryDatabaseContext, work: impl FnOnce(&Connection) -> Result<T, BackendError>) -> Result<T, BackendError> {
    let mut connection = open(app, database)?;
    let transaction = connection.transaction().map_err(storage)?;
    let value = work(&transaction)?;
    transaction.commit().map_err(storage)?;
    drop(connection);
    crate::database::sync_user_data_connection(app, database.android_directory_uri.as_deref()).map_err(storage)?;
    Ok(value)
}

fn read<T>(app: &AppHandle, database: &LibraryDatabaseContext, work: impl FnOnce(&Connection) -> Result<T, BackendError>) -> Result<T, BackendError> {
    work(&open(app, database)?)
}

fn task_from_json(text: &str) -> Result<AiTask, BackendError> {
    serde_json::from_str(text).map_err(|_| storage("una tarea guardada no se puede leer"))
}

fn task_json(task: &AiTask) -> Result<String, BackendError> {
    serde_json::to_string(task).map_err(storage)
}

fn insert_task(connection: &Connection, task: &AiTask) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO ai_tasks (id, status, created_at, updated_at, task_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task.id, task.status.id(), task.created_at_ms, task.updated_at_ms, task_json(task)?],
        )
        .map_err(storage)?;
    Ok(())
}

/// Saves a task that exists; false when it was deleted meanwhile.
fn update_task(connection: &Connection, task: &AiTask) -> Result<bool, BackendError> {
    let changed = connection
        .execute(
            "UPDATE ai_tasks SET status=?2, updated_at=?3, task_json=?4 WHERE id=?1",
            params![task.id, task.status.id(), task.updated_at_ms, task_json(task)?],
        )
        .map_err(storage)?;
    Ok(changed == 1)
}

fn load_task(connection: &Connection, id: &str) -> Result<Option<AiTask>, BackendError> {
    connection
        .query_row("SELECT task_json FROM ai_tasks WHERE id=?1", [id], |row| row.get::<_, String>(0))
        .optional()
        .map_err(storage)?
        .map(|text| task_from_json(&text))
        .transpose()
}

fn load_tasks(connection: &Connection) -> Result<Vec<AiTask>, BackendError> {
    let mut statement = connection.prepare("SELECT task_json FROM ai_tasks ORDER BY created_at DESC").map_err(storage)?;
    let texts = statement.query_map([], |row| row.get::<_, String>(0)).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?;
    // A task that cannot be read is left out instead of hiding the others.
    Ok(texts.iter().filter_map(|text| task_from_json(text).ok()).collect())
}

/// Deletes the tasks that did not change for a week and no runner holds;
/// a waiting task is kept while it waits.
fn delete_expired(connection: &Connection, now: i64) -> Result<usize, BackendError> {
    let held = runners().lock().map(|runners| runners.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let mut statement = connection.prepare("SELECT id FROM ai_tasks WHERE updated_at < ?1 AND status <> 'waiting'").map_err(storage)?;
    let expired = statement
        .query_map([now - RETENTION_MS], |row| row.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut deleted = 0;
    for id in expired.iter().filter(|id| !held.contains(id)) {
        deleted += connection.execute("DELETE FROM ai_tasks WHERE id=?1", [id]).map_err(storage)?;
    }
    Ok(deleted)
}

fn emit(app: &AppHandle, library_id: &str) {
    let _ = app.emit(TASK_CHANGED_EVENT, library_id);
}

/// The library's zone, for the panel's times.
fn library_zone(app: &AppHandle, library_id: &str) -> jiff::tz::TimeZone {
    let config = crate::library_config::read_library_config(app, library_id).ok().flatten();
    let place = crate::backend::weather::configured_location(config.as_ref()).0;
    time_zone(&crate::backend::ai_actions::library_time_zone(place.timezone.as_deref()))
}

// ---------------------------------------------------------------------------
// Runners and approvals of this process
// ---------------------------------------------------------------------------

/// What this process knows of a task it works.
struct Runner {
    cancel: Arc<AtomicBool>,
    /// The run of the phase that works now, to cancel it and show its thinking.
    current: Option<CurrentRun>,
    /// The approval it waits for and where its answer goes.
    waiting: Option<WaitingApproval>,
}

struct WaitingApproval {
    approval_id: String,
    /// Its message in the Owner's Telegram chat: a reply to it answers.
    telegram: Option<(i64, i64)>,
    kind: ApprovalKind,
    sender: mpsc::Sender<ApprovalAnswer>,
}

#[derive(Clone)]
struct CurrentRun {
    context: BackendRequestContext,
    idempotency_key: String,
    phase: AiTaskPhase,
    step_id: Option<String>,
    started_ms: i64,
}

fn runners() -> &'static Mutex<HashMap<String, Runner>> {
    static RUNNERS: OnceLock<Mutex<HashMap<String, Runner>>> = OnceLock::new();
    RUNNERS.get_or_init(Default::default)
}

fn with_runner<T>(task_id: &str, work: impl FnOnce(&mut Runner) -> T) -> Option<T> {
    runners().lock().ok()?.get_mut(task_id).map(work)
}

/// What the person chose for an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnswerChoice {
    Confirm,
    ConfirmAll,
    Propose,
    Cancel,
    /// The answer to a question: `text` or `optionIndex`.
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswer {
    pub choice: AnswerChoice,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub option_index: Option<usize>,
}

fn invalid_answer(message: &str) -> BackendError {
    BackendError::invalid_input(message)
}

/// The decision a run resumes with, and the text that marks the approval
/// answered. Choosing «Confirmar todos» also approves the task's next changes.
fn decision_for(
    interaction: &PendingInteraction,
    operation: &OperationToken,
    answer: &ApprovalAnswer,
) -> Result<(ResumeDecision, String), BackendError> {
    let proposal = match answer.choice {
        AnswerChoice::Propose => Some(
            answer
                .text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| invalid_answer("Escribí qué querés que haga en su lugar."))?
                .to_string(),
        ),
        _ => None,
    };
    let accepted = matches!(answer.choice, AnswerChoice::Confirm | AnswerChoice::ConfirmAll);
    let approve_all = answer.choice == AnswerChoice::ConfirmAll;
    let outcome = match answer.choice {
        AnswerChoice::Confirm => "Confirmado.",
        AnswerChoice::ConfirmAll => "Confirmado todo lo que queda de la tarea.",
        AnswerChoice::Propose => "Propuesta enviada.",
        AnswerChoice::Cancel => "Rechazado.",
        AnswerChoice::Answer => "Respondido.",
    };
    match interaction {
        PendingInteraction::Confirmation(_) if answer.choice != AnswerChoice::Answer => Ok((
            ResumeDecision::Confirmation(crate::backend::ConfirmationDecision {
                operation_id: operation.operation_id.clone(),
                accepted,
                hunk_ids: Vec::new(),
                approve_all,
                suggestion: proposal,
            }),
            outcome.to_string(),
        )),
        PendingInteraction::Plan(plan) if answer.choice != AnswerChoice::Answer => Ok((
            ResumeDecision::Plan(crate::backend::PlanDecision {
                plan_id: plan.plan_id.clone(),
                generation: plan.generation,
                accepted,
                step_ids: plan.steps.iter().map(|step| step.id.clone()).collect(),
                suggestion: proposal,
                approve_all,
            }),
            outcome.to_string(),
        )),
        PendingInteraction::Clarification(request) if answer.choice == AnswerChoice::Answer => {
            let option = answer.option_index.and_then(|index| request.options.get(index));
            let text = match (option, answer.text.as_deref().map(str::trim).filter(|text| !text.is_empty())) {
                (Some(option), _) => option.label.clone(),
                (None, Some(text)) => text.to_string(),
                (None, None) => return Err(invalid_answer("Escribí la respuesta o elegí una opción.")),
            };
            Ok((
                ResumeDecision::Clarification(crate::backend::ClarificationAnswer {
                    clarification_id: request.clarification_id.clone(),
                    operation: operation.clone(),
                    answer: text.clone(),
                    option_id: option.map(|option| option.id.clone()),
                }),
                format!("Respondido: {}", clipped(&text, 200)),
            ))
        }
        PendingInteraction::Clarification(_) => Err(invalid_answer("La pregunta se responde con un texto o una opción.")),
        _ => Err(invalid_answer("Esa respuesta no corresponde a lo que la tarea espera.")),
    }
}

/// The approval a run waits for, as the panel and Telegram show it.
fn approval_of(interaction: &PendingInteraction, phase: AiTaskPhase, step_id: Option<String>) -> AiTaskApproval {
    let id = Uuid::new_v4().simple().to_string()[..8].to_string();
    let (kind, title, detail, options) = match interaction {
        PendingInteraction::Confirmation(request) => {
            let detail = request
                .preview
                .hunks
                .iter()
                .map(|hunk| hunk.new_text.trim().to_string())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (ApprovalKind::Confirmation, request.preview.summary.clone(), clipped(&detail, 6_000), Vec::new())
        }
        PendingInteraction::Plan(plan) => {
            let steps = plan.steps.iter().enumerate().map(|(index, step)| format!("{}. {}", index + 1, step.label)).collect::<Vec<_>>().join("\n");
            (ApprovalKind::Plan, plan.title.clone(), steps, Vec::new())
        }
        PendingInteraction::Clarification(request) => (
            ApprovalKind::Clarification,
            request.question.clone(),
            String::new(),
            request.options.iter().map(|option| option.label.clone()).collect(),
        ),
    };
    AiTaskApproval { id, phase, step_id, kind, title, detail, options, created_at_ms: now_ms(), telegram_message_id: None }
}

/// The Telegram buttons of an approval: the choices of a change or a plan,
/// or the options of a question.
fn telegram_buttons(approval: &AiTaskApproval) -> Vec<(String, String)> {
    let data = |choice: &str| format!("{TELEGRAM_CALLBACK_PREFIX}{}:{choice}", approval.id);
    match approval.kind {
        ApprovalKind::Clarification => approval
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| (clipped(option, crate::backend::telegram_bot::MAX_BUTTON_CHARS), data(&format!("o{index}"))))
            .collect(),
        ApprovalKind::Confirmation | ApprovalKind::Plan => vec![
            ("Confirmar".to_string(), data("c")),
            ("Confirmar todos".to_string(), data("a")),
            ("Cancelar".to_string(), data("x")),
        ],
    }
}

/// A button of a task's approval tapped in Telegram by the Owner.
pub(crate) fn answer_from_telegram(data: &str) {
    let Some(rest) = data.strip_prefix(TELEGRAM_CALLBACK_PREFIX) else { return };
    let Some((approval_id, choice)) = rest.split_once(':') else { return };
    let answer = match choice {
        "c" => ApprovalAnswer { choice: AnswerChoice::Confirm, text: None, option_index: None },
        "a" => ApprovalAnswer { choice: AnswerChoice::ConfirmAll, text: None, option_index: None },
        "x" => ApprovalAnswer { choice: AnswerChoice::Cancel, text: None, option_index: None },
        option => match option.strip_prefix('o').and_then(|index| index.parse::<usize>().ok()) {
            Some(index) => ApprovalAnswer { choice: AnswerChoice::Answer, text: None, option_index: Some(index) },
            None => return,
        },
    };
    if let Ok(runners) = runners().lock() {
        if let Some(waiting) = runners.values().filter_map(|runner| runner.waiting.as_ref()).find(|waiting| waiting.approval_id == approval_id) {
            let _ = waiting.sender.send(answer);
        }
    }
}

/// A text the Owner wrote in Telegram as a reply («Responder») to the
/// message of a task's approval: it answers a question, or proposes
/// something else instead of a change or a plan. Whether it was one.
pub(crate) fn answer_reply_from_telegram(chat_id: i64, reply_to_message_id: i64, text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let Ok(runners) = runners().lock() else { return false };
    let Some(waiting) = runners
        .values()
        .filter_map(|runner| runner.waiting.as_ref())
        .find(|waiting| waiting.telegram == Some((chat_id, reply_to_message_id)))
    else {
        return false;
    };
    let choice = if waiting.kind == ApprovalKind::Clarification { AnswerChoice::Answer } else { AnswerChoice::Propose };
    waiting.sender.send(ApprovalAnswer { choice, text: Some(text.to_string()), option_index: None }).is_ok()
}

/// The Telegram chat where a task talks to the Owner: the one it was
/// created in, or the chat the Owner linked, wherever the task was created
/// (the app, an AI action); `None` while the Owner has no Telegram linked.
fn owner_chat(app: &AppHandle, database: &LibraryDatabaseContext, task: &AiTask) -> Option<i64> {
    match (task.origin, task.telegram_chat_id) {
        (AiTaskOrigin::Telegram, Some(chat_id)) => Some(chat_id),
        _ => crate::library_users::owner_telegram_link(app, database).ok().flatten().map(|(_, chat_id)| chat_id),
    }
}

// ---------------------------------------------------------------------------
// Running a task
// ---------------------------------------------------------------------------

/// Why a phase stopped without an answer.
enum PhaseStop {
    Cancelled,
    /// The task was deleted while it worked.
    Gone,
    Failed(String),
    /// The provider failed or answered nothing: the same work is tried
    /// again (`MAX_RUN_RETRIES`) before the task fails.
    Interrupted(String),
}

/// A run that ended for the provider (down, a server failure, an empty
/// answer) is tried again; any other failure stops the task.
fn run_failure(error: BackendError) -> PhaseStop {
    if matches!(error.code, BackendErrorCode::ProviderUnavailable | BackendErrorCode::Timeout) {
        PhaseStop::Interrupted(error.message)
    } else {
        PhaseStop::Failed(error.message)
    }
}

/// Retries of one phase (the plan, a step, the review) whose run was cut or,
/// for a step, ended without its report; the steps done stay.
const MAX_RUN_RETRIES: u32 = 2;
const RUN_RETRY_PAUSE: Duration = Duration::from_secs(5);

/// The step goes back to be done again, with how the last try ended.
fn retry_step(task: &mut AiTask, index: usize, report: &str) {
    let step = &mut task.steps[index];
    step.status = StepStatus::Failed;
    step.report = Some(clipped_report(report));
    step.finished_at_ms = Some(now_ms());
}

/// Waits before trying a phase again; a cancel ends the wait (the loop
/// then finishes the task as cancelled).
fn pause_before_retry(cancel: &AtomicBool) {
    let until = Instant::now() + RUN_RETRY_PAUSE;
    while Instant::now() < until && !cancel.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Starts working a stored task in a thread of this process, unless one
/// already works it.
fn spawn(app: &AppHandle, library_id: &str, task_id: &str) {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut runners) = runners().lock() else { return };
        if runners.contains_key(task_id) {
            return;
        }
        runners.insert(task_id.to_string(), Runner { cancel: Arc::clone(&cancel), current: None, waiting: None });
    }
    let (app, library_id, task_id) = (app.clone(), library_id.to_string(), task_id.to_string());
    let spawned = std::thread::Builder::new().name("notia-ai-task".into()).spawn({
        let task_id = task_id.clone();
        move || {
            if let Err(error) = work(&app, &library_id, &task_id, &cancel) {
                log::error!("[notia:ai-tasks] la tarea se detuvo por un error de almacenamiento: {:?}", error.code);
            }
            if let Ok(mut runners) = runners().lock() {
                runners.remove(&task_id);
            }
            emit(&app, &library_id);
        }
    });
    if spawned.is_err() {
        if let Ok(mut runners) = runners().lock() {
            runners.remove(&task_id);
        }
    }
}

/// Saves the task as it is now; false when it was deleted.
fn save(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, task: &mut AiTask) -> Result<bool, BackendError> {
    task.updated_at_ms = now_ms();
    let saved = write(app, database, |connection| update_task(connection, task))?;
    emit(app, library_id);
    Ok(saved)
}

/// Ends the task with `status` and tells Telegram, for a task created there.
fn finish(app: &AppHandle, library_id: &str, database: &LibraryDatabaseContext, task: &mut AiTask, status: AiTaskStatus, error: Option<String>) -> Result<(), BackendError> {
    task.status = status;
    task.phase = None;
    task.pending = None;
    task.error = error;
    task.finished_at_ms = Some(now_ms());
    task.wait = None;
    if let Some(step) = task.steps.iter_mut().find(|step| matches!(step.status, StepStatus::Running | StepStatus::Waiting)) {
        step.status = if status == AiTaskStatus::Completed { StepStatus::Done } else { StepStatus::Failed };
    }
    if !save(app, library_id, database, task)? {
        return Ok(());
    }
    if status != AiTaskStatus::Cancelled {
        if let Some(chat_id) = owner_chat(app, database, task) {
            crate::telegram_worker::send_task_message(app, library_id, chat_id, &finished_message(task), Vec::new());
        }
    }
    Ok(())
}

/// Works the task until it ends, is cancelled or waits too long.
fn work(app: &AppHandle, library_id: &str, task_id: &str, cancel: &AtomicBool) -> Result<(), BackendError> {
    let database = database(app, library_id)?;
    let mut retries = 0;
    loop {
        let Some(mut task) = read(app, &database, |connection| load_task(connection, task_id))? else {
            return Ok(());
        };
        if cancel.load(Ordering::SeqCst) {
            return finish(app, library_id, &database, &mut task, AiTaskStatus::Cancelled, None);
        }
        let next = task.next_work();
        let (phase, step_index, request) = match next {
            NextWork::Done => return finish(app, library_id, &database, &mut task, AiTaskStatus::Completed, None),
            NextWork::Plan => (AiTaskPhase::Planner, None, planner_request(&task)),
            NextWork::Execute(index) => (AiTaskPhase::Executor, Some(index), executor_request(&task, index)),
            NextWork::Review => (AiTaskPhase::Review, None, review_request(&task)),
        };
        task.status = match phase {
            AiTaskPhase::Planner => AiTaskStatus::Planning,
            AiTaskPhase::Executor => AiTaskStatus::Running,
            AiTaskPhase::Review => AiTaskStatus::Reviewing,
        };
        task.phase = Some(phase);
        task.error = None;
        if let Some(index) = step_index {
            task.steps[index].status = StepStatus::Running;
            task.steps[index].started_at_ms = Some(now_ms());
        }
        if !save(app, library_id, &database, &mut task)? {
            return Ok(());
        }
        let step_id = step_index.map(|index| task.steps[index].id.clone());
        let answer = match run_phase(app, library_id, &database, task_id, phase, step_id, request, cancel) {
            Ok(answer) => answer,
            Err(PhaseStop::Gone) => return Ok(()),
            Err(stop) => {
                let Some(mut task) = read(app, &database, |connection| load_task(connection, task_id))? else {
                    return Ok(());
                };
                return match stop {
                    PhaseStop::Interrupted(error) if retries < MAX_RUN_RETRIES => {
                        retries += 1;
                        log::warn!("[notia:ai-task] {} se cortó ({error}); intento {} de {}", phase.label(), retries + 1, MAX_RUN_RETRIES + 1);
                        if let Some(index) = step_index {
                            retry_step(&mut task, index, &format!("No terminado: la corrida se cortó ({error})."));
                        }
                        if !save(app, library_id, &database, &mut task)? {
                            return Ok(());
                        }
                        pause_before_retry(cancel);
                        continue;
                    }
                    PhaseStop::Cancelled => finish(app, library_id, &database, &mut task, AiTaskStatus::Cancelled, None),
                    PhaseStop::Failed(error) | PhaseStop::Interrupted(error) => {
                        finish(app, library_id, &database, &mut task, AiTaskStatus::Failed, Some(format!("{}: {error}", phase.label())))
                    }
                    PhaseStop::Gone => Ok(()),
                };
            }
        };
        // The run may have changed the task (an approval, «Confirmar todos»).
        let Some(mut task) = read(app, &database, |connection| load_task(connection, task_id))? else {
            return Ok(());
        };
        let now = now_ms();
        match next {
            NextWork::Plan => {
                let steps = parse_plan(&answer, &task.goal);
                task.set_plan(steps);
            }
            NextWork::Execute(index) => {
                // The note of the last wait went in this run's request.
                task.wake = None;
                if task.wait.as_ref().is_some_and(|wait| wait.step_id == task.steps[index].id) {
                    // The step waits: the thread ends and the watch wakes it.
                    waits::begin_wait(&mut task, index, &answer);
                    save(app, library_id, &database, &mut task)?;
                    // A reply that arrived during the run wakes it now.
                    look_at_waits_at(Instant::now() + LOOK_MARGIN);
                    return Ok(());
                }
                task.wait = None;
                if step_without_report(&answer) {
                    if retries < MAX_RUN_RETRIES {
                        retries += 1;
                        log::warn!("[notia:ai-task] el paso {} terminó sin informe; intento {} de {}", index + 1, retries + 1, MAX_RUN_RETRIES + 1);
                        retry_step(&mut task, index, &answer);
                        if !save(app, library_id, &database, &mut task)? {
                            return Ok(());
                        }
                        continue;
                    }
                    retry_step(&mut task, index, &answer);
                    let error = format!("El paso {} no tuvo informe en {} intentos.", index + 1, MAX_RUN_RETRIES + 1);
                    return finish(app, library_id, &database, &mut task, AiTaskStatus::Failed, Some(error));
                }
                let step = &mut task.steps[index];
                step.report = Some(clipped_report(&answer));
                step.finished_at_ms = Some(now);
                if step_unfinished(&answer) {
                    step.status = StepStatus::Failed;
                    let error = format!("El paso {} no se pudo terminar.", index + 1);
                    return finish(app, library_id, &database, &mut task, AiTaskStatus::Failed, Some(error));
                }
                step.status = StepStatus::Done;
            }
            NextWork::Review => match task.apply_review(parse_review(&answer), now) {
                ReviewEffect::Approved => return finish(app, library_id, &database, &mut task, AiTaskStatus::Completed, None),
                ReviewEffect::Rework => {}
                ReviewEffect::Exhausted => {
                    let error = "La revisión pidió rehacer la tarea demasiadas veces; revisala y reanudala o pedime otra revisión.".to_string();
                    return finish(app, library_id, &database, &mut task, AiTaskStatus::Failed, Some(error));
                }
            },
            NextWork::Done => {}
        }
        retries = 0;
        if !save(app, library_id, &database, &mut task)? {
            return Ok(());
        }
    }
}

/// Context of a phase's run: a request of the Owner, in the app, with
/// memory, like a chat of the library.
fn phase_context(library_id: &str) -> BackendRequestContext {
    BackendRequestContext {
        request_id: format!("ai-task-{}", Uuid::new_v4().simple()),
        library_id: library_id.to_string(),
        actor: BackendActor { library_user_id: crate::backend::OWNER_LIBRARY_USER_ID.to_string(), external_identity: None },
        channel: BackendChannel::App,
        scope: BackendScope::Library,
        persistence_policy: PersistencePolicy::Persistent,
    }
}

/// Runs one phase until the agent answers; each pause is approved by
/// «Confirmar todos» or waits for the Owner. The answer's Markdown.
#[allow(clippy::too_many_arguments)]
fn run_phase(
    app: &AppHandle,
    library_id: &str,
    database: &LibraryDatabaseContext,
    task_id: &str,
    phase: AiTaskPhase,
    step_id: Option<String>,
    text: String,
    cancel: &AtomicBool,
) -> Result<String, PhaseStop> {
    let runtime = app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
    if let Some(config) = crate::library_config::read_library_config(app, library_id).ok().flatten() {
        runtime.configure_from_library_config(&config).map_err(|error| PhaseStop::Failed(error.message))?;
    }
    let context = phase_context(library_id);
    let idempotency_key = format!("{library_id}:{}", context.request_id);
    with_runner(task_id, |runner| {
        runner.current = Some(CurrentRun {
            context: context.clone(),
            idempotency_key: idempotency_key.clone(),
            phase,
            step_id: step_id.clone(),
            started_ms: now_ms(),
        })
    });
    let mut request = BackendRequest::Run(AgentRequest {
        context: context.clone(),
        messages: vec![BackendMessage { role: MessageRole::User, content: text, images: Vec::new(), attachments: Vec::new() }],
        snapshot: None,
        tools: Vec::new(),
        attachments: Vec::new(),
        idempotency_key: idempotency_key.clone(),
        prompt_name: Some(phase.prompt_file().to_string()),
        tool_access: Default::default(),
        library_search: true,
        autonomous: false,
        scheduled_action: None,
        webhook_call: None,
        tool_areas: None,
        thinking: None,
        conversational: false,
        ai_task: Some(AiTaskTurn { task_id: task_id.to_string(), phase }),
    });
    let result = loop {
        let registry = app.state::<LibraryBindingRegistry>();
        let response = crate::backend_runtime::execute_backend_request(
            app,
            BackendRequestEnvelope { protocol_version: ProtocolVersion::default(), request },
            &runtime,
            registry.inner(),
        );
        if cancel.load(Ordering::SeqCst) {
            break Err(PhaseStop::Cancelled);
        }
        let response = match response {
            Ok(envelope) => envelope.response,
            Err(error) => {
                crate::backend_runtime::log_request_failure(&runtime, "ai-task", &context, &error);
                break Err(run_failure(error));
            }
        };
        match response {
            BackendResponse::Result { response } => {
                if response.changed {
                    let _ = app.emit(crate::telegram_worker::LIBRARY_CHANGED_EVENT, library_id);
                }
                break Ok(response.response.markdown);
            }
            BackendResponse::Error { error, .. } => {
                crate::backend_runtime::log_request_failure(&runtime, "ai-task", &context, &error);
                break Err(run_failure(error));
            }
            BackendResponse::Operation { status } | BackendResponse::Resumed { status, .. } => {
                let (Some(operation), Some(interaction)) = (status.operation.clone(), status.interaction.clone()) else {
                    break Err(PhaseStop::Failed("El runtime no devolvió una respuesta final.".to_string()));
                };
                let decision = match approve(app, library_id, database, task_id, phase, step_id.clone(), &interaction, &operation, cancel) {
                    Ok(decision) => decision,
                    Err(stop) => {
                        cancel_operation(app, &runtime, &context, &idempotency_key, operation);
                        break Err(stop);
                    }
                };
                request = BackendRequest::Resume(ResumeRequest {
                    context: context.clone(),
                    idempotency_key: idempotency_key.clone(),
                    request_id: context.request_id.clone(),
                    operation,
                    last_event_sequence: status.last_event_sequence,
                    decision,
                });
            }
            _ => break Err(PhaseStop::Failed("El runtime no devolvió una respuesta final.".to_string())),
        }
    };
    with_runner(task_id, |runner| runner.current = None);
    result
}

fn cancel_operation(
    app: &AppHandle,
    runtime: &crate::backend_runtime::BackendRuntimeState,
    context: &BackendRequestContext,
    idempotency_key: &str,
    operation: OperationToken,
) {
    let registry = app.state::<LibraryBindingRegistry>();
    let _ = crate::backend_runtime::execute_backend_request(app, cancel_envelope(context, idempotency_key, Some(operation)), runtime, registry.inner());
}

fn cancel_envelope(context: &BackendRequestContext, idempotency_key: &str, operation: Option<OperationToken>) -> BackendRequestEnvelope {
    BackendRequestEnvelope {
        protocol_version: ProtocolVersion::default(),
        request: BackendRequest::Cancel(crate::backend::CancelRequest {
            context: context.clone(),
            idempotency_key: idempotency_key.to_string(),
            request_id: context.request_id.clone(),
            operation,
        }),
    }
}

/// The decision for a pause of a phase: with «Confirmar todos» a change or
/// a plan is approved at once; otherwise the task waits for the Owner, in
/// the panel and, for a task created on Telegram, in the Owner's chat.
#[allow(clippy::too_many_arguments)]
fn approve(
    app: &AppHandle,
    library_id: &str,
    database: &LibraryDatabaseContext,
    task_id: &str,
    phase: AiTaskPhase,
    step_id: Option<String>,
    interaction: &PendingInteraction,
    operation: &OperationToken,
    cancel: &AtomicBool,
) -> Result<ResumeDecision, PhaseStop> {
    let stored = |error: BackendError| PhaseStop::Failed(error.message);
    let mut task = read(app, database, |connection| load_task(connection, task_id)).map_err(stored)?.ok_or(PhaseStop::Gone)?;
    let approves = !matches!(interaction, PendingInteraction::Clarification(_));
    if task.approve_all && approves {
        let answer = ApprovalAnswer { choice: AnswerChoice::ConfirmAll, text: None, option_index: None };
        return decision_for(interaction, operation, &answer).map(|(decision, _)| decision).map_err(stored);
    }
    let mut approval = approval_of(interaction, phase, step_id);
    let previous_status = task.status;
    let chat = owner_chat(app, database, &task);
    if let Some(chat_id) = chat {
        approval.telegram_message_id =
            crate::telegram_worker::send_task_message(app, library_id, chat_id, &approval_message(&task, &approval), telegram_buttons(&approval));
    }
    let (sender, receiver) = mpsc::channel();
    let waiting = WaitingApproval {
        approval_id: approval.id.clone(),
        telegram: chat.zip(approval.telegram_message_id),
        kind: approval.kind,
        sender,
    };
    with_runner(task_id, |runner| runner.waiting = Some(waiting));
    task.pending = Some(approval.clone());
    task.status = AiTaskStatus::AwaitingApproval;
    if !save(app, library_id, database, &mut task).map_err(stored)? {
        return Err(PhaseStop::Gone);
    }
    let started = Instant::now();
    let outcome = loop {
        match receiver.recv_timeout(APPROVAL_POLL) {
            Ok(answer) => match decision_for(interaction, operation, &answer) {
                Ok((decision, outcome)) => break Ok((answer, decision, outcome)),
                // A wrong answer from the panel was refused there already.
                Err(_) => continue,
            },
            Err(mpsc::RecvTimeoutError::Timeout) if cancel.load(Ordering::SeqCst) => break Err(PhaseStop::Cancelled),
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() > APPROVAL_TIMEOUT => {
                break Err(PhaseStop::Failed("nadie respondió la aprobación en un día.".to_string()))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(PhaseStop::Cancelled),
        }
    };
    with_runner(task_id, |runner| runner.waiting = None);
    if let (Some(chat_id), Some(message_id)) = (chat, approval.telegram_message_id) {
        let mark = match &outcome {
            Ok((_, _, outcome)) => outcome.clone(),
            Err(_) => "Sin respuesta.".to_string(),
        };
        let html = format!(
            "<b>Tarea «{}»</b>\n{}\n\n<i>{}</i>",
            crate::backend::formatting::escape_telegram_html(&task.title),
            crate::backend::formatting::escape_telegram_html(&clipped(&approval.title, 1_000)),
            crate::backend::formatting::escape_telegram_html(&mark)
        );
        crate::telegram_worker::mark_task_message(app, library_id, chat_id, message_id, &html);
    }
    let (answer, decision, _) = outcome?;
    let mut task = read(app, database, |connection| load_task(connection, task_id)).map_err(stored)?.ok_or(PhaseStop::Gone)?;
    task.pending = None;
    task.status = previous_status;
    if answer.choice == AnswerChoice::ConfirmAll {
        task.approve_all = true;
    }
    if !save(app, library_id, database, &mut task).map_err(stored)? {
        return Err(PhaseStop::Gone);
    }
    Ok(decision)
}

// ---------------------------------------------------------------------------
// Actions shared by the panel and the agent's tools
// ---------------------------------------------------------------------------

/// Creates a task and starts working it.
fn create(app: &AppHandle, library_id: &str, context: &BackendRequestContext, new: NewTask, approve_all: bool) -> Result<AiTask, BackendError> {
    let database = database(app, library_id)?;
    let telegram_chat_id = context.actor.external_identity.as_ref().filter(|identity| identity.provider == "telegram").and_then(|identity| identity.chat_id);
    let origin = if context.channel == BackendChannel::Telegram { AiTaskOrigin::Telegram } else { AiTaskOrigin::App };
    let task = AiTask::new(Uuid::new_v4().to_string(), &new.title, &new.goal, origin, telegram_chat_id, approve_all, now_ms());
    write(app, &database, |connection| {
        delete_expired(connection, task.created_at_ms)?;
        insert_task(connection, &task)
    })?;
    emit(app, library_id);
    spawn(app, library_id, &task.id);
    Ok(task)
}

fn load(app: &AppHandle, library_id: &str, task_id: &str) -> Result<AiTask, BackendError> {
    let database = database(app, library_id)?;
    read(app, &database, |connection| load_task(connection, task_id))?.ok_or_else(not_found)
}

/// How long a cancel waits for the run at work to let go of the task, so
/// the list read right after shows it cancelled.
const STOP_WAIT: Duration = Duration::from_secs(20);

/// Stops a task: the run of its phase is cancelled and the runner is waited
/// for (up to `STOP_WAIT`); a task no runner holds is marked cancelled at
/// once. Returns whether it already stopped.
fn cancel(app: &AppHandle, library_id: &str, task_id: &str) -> Result<bool, BackendError> {
    let mut task = load(app, library_id, task_id)?;
    tools::check_cancellable(&task)?;
    stop(app, library_id, &mut task)
}

fn stop(app: &AppHandle, library_id: &str, task: &mut AiTask) -> Result<bool, BackendError> {
    let task_id = task.id.clone();
    let current = with_runner(&task_id, |runner| {
        runner.cancel.store(true, Ordering::SeqCst);
        runner.current.clone()
    });
    match current {
        Some(Some(run)) => {
            let runtime = app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
            let registry = app.state::<LibraryBindingRegistry>();
            let _ = crate::backend_runtime::execute_backend_request(app, cancel_envelope(&run.context, &run.idempotency_key, None), &runtime, registry.inner());
        }
        Some(None) => {}
        None => {
            let database = database(app, library_id)?;
            finish(app, library_id, &database, task, AiTaskStatus::Cancelled, None)?;
            return Ok(true);
        }
    }
    let deadline = Instant::now() + STOP_WAIT;
    while with_runner(&task_id, |_| ()).is_some() {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(true)
}

/// Goes on with a task that stopped, from its first step not done; a
/// waiting one goes on without waiting more. With a correction, a task at
/// work stops first, then its goal changes and it plans again.
fn resume(app: &AppHandle, library_id: &str, task_id: &str, replan: Option<&tools::Replan>) -> Result<AiTask, BackendError> {
    let mut task = load(app, library_id, task_id)?;
    tools::check_resumable(&task, replan)?;
    if let Some(replan) = replan {
        if task.status.is_active() && task.status != AiTaskStatus::Waiting {
            if !stop(app, library_id, &mut task)? {
                return Err(BackendError::new(
                    BackendErrorCode::Conflict,
                    "La tarea sigue terminando lo que hacía; probá de nuevo en unos segundos.",
                    true,
                ));
            }
            task = load(app, library_id, task_id)?;
        }
        task.replan(replan.goal.as_deref(), replan.restart);
    } else if task.status == AiTaskStatus::Waiting {
        waits::wake(&mut task, waits::RESUMED_NOTE);
    }
    if replan.is_none() {
        task.status = if task.steps.is_empty() { AiTaskStatus::Planning } else { AiTaskStatus::Running };
    }
    task.error = None;
    task.finished_at_ms = None;
    let database = database(app, library_id)?;
    save(app, library_id, &database, &mut task)?;
    spawn(app, library_id, task_id);
    Ok(task)
}

/// Runs another review of a task that ended; a rework works it again.
fn review_again(app: &AppHandle, library_id: &str, task_id: &str, instructions: Option<String>) -> Result<AiTask, BackendError> {
    let mut task = load(app, library_id, task_id)?;
    tools::check_reviewable(&task)?;
    task.review_request = Some(instructions.unwrap_or_else(|| "Revisá de nuevo que el objetivo esté cumplido.".to_string()));
    task.status = AiTaskStatus::Reviewing;
    task.error = None;
    task.result = None;
    task.finished_at_ms = None;
    let database = database(app, library_id)?;
    save(app, library_id, &database, &mut task)?;
    spawn(app, library_id, task_id);
    Ok(task)
}

fn delete(app: &AppHandle, library_id: &str, task_id: &str) -> Result<AiTask, BackendError> {
    let task = load(app, library_id, task_id)?;
    with_runner(task_id, |runner| runner.cancel.store(true, Ordering::SeqCst));
    let database = database(app, library_id)?;
    write(app, &database, |connection| connection.execute("DELETE FROM ai_tasks WHERE id=?1", [task_id]).map_err(storage))?;
    emit(app, library_id);
    Ok(task)
}

/// Sends the Owner's answer to the approval a task waits for.
fn answer(task_id: &str, approval_id: &str, answer: ApprovalAnswer) -> Result<(), BackendError> {
    let sent = with_runner(task_id, |runner| match runner.waiting.as_ref() {
        Some(waiting) if waiting.approval_id == approval_id => waiting.sender.send(answer).is_ok(),
        _ => false,
    });
    if sent == Some(true) {
        Ok(())
    } else {
        Err(BackendError::new(BackendErrorCode::Conflict, "La tarea ya no espera esa respuesta.", false))
    }
}

// ---------------------------------------------------------------------------
// Panel commands
// ---------------------------------------------------------------------------

/// Library of a panel command; the tasks are the Owner's only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTasksContext {
    pub library_id: String,
    pub actor_library_user_id: String,
}

impl AiTasksContext {
    fn library(&self) -> Result<&str, BackendError> {
        if self.actor_library_user_id.trim() != crate::backend::OWNER_LIBRARY_USER_ID {
            return Err(owner_only());
        }
        Ok(&self.library_id)
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, BackendError> + Send + 'static) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(work).await.map_err(|_| storage("la operación no terminó"))?
}

pub async fn ai_tasks_list(app: AppHandle, context: AiTasksContext) -> Result<Vec<AiTaskCard>, BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        let database = database(&app, &library_id)?;
        let now = now_ms();
        let tasks = write(&app, &database, |connection| {
            delete_expired(connection, now)?;
            load_tasks(connection)
        })?;
        Ok(views::cards(&tasks, &library_zone(&app, &library_id), now))
    })
    .await
}

pub async fn ai_task_get(app: AppHandle, context: AiTasksContext, task_id: String) -> Result<AiTaskDetail, BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        let task = load(&app, &library_id, &task_id)?;
        Ok(views::detail(&task, &library_zone(&app, &library_id), now_ms()))
    })
    .await
}

/// The thinking of the phase the task works now, as it streamed so far;
/// `None` when no phase runs.
pub async fn ai_task_thinking(app: AppHandle, context: AiTasksContext, task_id: String) -> Result<Option<AiTaskThinking>, BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        let task = load(&app, &library_id, &task_id)?;
        let Some(run) = with_runner(&task_id, |runner| runner.current.clone()).flatten() else {
            return Ok(None);
        };
        let runtime = app.state::<crate::backend_runtime::BackendRuntimeState>();
        let events = runtime.request_events(&run.context, 0).unwrap_or_default();
        Ok(Some(views::thinking(&task, &run.context.request_id, run.phase, run.step_id, &events)))
    })
    .await
}

pub async fn ai_task_answer(app: AppHandle, context: AiTasksContext, task_id: String, approval_id: String, answer_input: ApprovalAnswer) -> Result<(), BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        let task = load(&app, &library_id, &task_id)?;
        let pending = task.pending.as_ref().filter(|pending| pending.id == approval_id).ok_or_else(|| {
            BackendError::new(BackendErrorCode::Conflict, "La tarea ya no espera esa respuesta.", false)
        })?;
        // The answer is checked here, so a wrong one is refused in the panel.
        match (pending.kind, answer_input.choice) {
            (ApprovalKind::Clarification, AnswerChoice::Answer) => {
                let has_option = answer_input.option_index.is_some_and(|index| index < pending.options.len());
                let has_text = answer_input.text.as_deref().is_some_and(|text| !text.trim().is_empty());
                if !has_option && !has_text {
                    return Err(invalid_answer("Escribí la respuesta o elegí una opción."));
                }
            }
            (ApprovalKind::Clarification, _) => return Err(invalid_answer("La pregunta se responde con un texto o una opción.")),
            (_, AnswerChoice::Answer) => return Err(invalid_answer("Elegí confirmar, confirmar todos, proponer otra cosa o cancelar.")),
            (_, AnswerChoice::Propose) if answer_input.text.as_deref().is_none_or(|text| text.trim().is_empty()) => {
                return Err(invalid_answer("Escribí qué querés que haga en su lugar."))
            }
            _ => {}
        }
        answer(&task_id, &approval_id, answer_input)
    })
    .await
}

pub async fn ai_task_cancel(app: AppHandle, context: AiTasksContext, task_id: String) -> Result<(), BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        cancel(&app, &library_id, &task_id).map(|_| ())
    })
    .await
}

pub async fn ai_task_resume(app: AppHandle, context: AiTasksContext, task_id: String) -> Result<(), BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        resume(&app, &library_id, &task_id, None).map(|_| ())
    })
    .await
}

pub async fn ai_task_delete(app: AppHandle, context: AiTasksContext, task_id: String) -> Result<(), BackendError> {
    blocking(move || {
        let library_id = context.library()?.to_string();
        delete(&app, &library_id, &task_id).map(|_| ())
    })
    .await
}

// ---------------------------------------------------------------------------
// Agent tools
// ---------------------------------------------------------------------------

pub(crate) use crate::backend::ai_tasks::tools::{is_ai_task_tool, is_ai_task_write_tool};

fn tasks_of(app: &AppHandle, library_id: &str) -> Result<Vec<AiTask>, BackendError> {
    let database = database(app, library_id)?;
    read(app, &database, load_tasks)
}

/// The confirmation of a write tool, checked as its execution will do it.
pub(crate) fn preview_tool(app: &AppHandle, context: &BackendRequestContext, name: &str, arguments: &Value) -> Result<String, BackendError> {
    if name == "create_ai_task" {
        return Ok(tools::create_summary(&tools::new_task(arguments)?));
    }
    let tasks = tasks_of(app, &context.library_id)?;
    let task = tools::resolve_task(&tasks, arguments)?;
    match name {
        "cancel_ai_task" => tools::check_cancellable(task).map(|()| tools::cancel_summary(task)),
        "resume_ai_task" => {
            let replan = tools::replan_request(arguments)?;
            tools::check_resumable(task, replan.as_ref()).map(|()| tools::resume_summary(task, replan.as_ref()))
        }
        "review_ai_task" => tools::check_reviewable(task).map(|()| tools::review_summary(task, tools::review_instructions(arguments).as_deref())),
        "delete_ai_task" => Ok(tools::delete_summary(task)),
        _ => Err(BackendError::invalid_input("La herramienta no es de Tareas IA.")),
    }
}

/// Runs a task tool: a read, or a write the Owner confirmed. `approveAll`
/// of `create_ai_task` is the confirmation's «Confirmar todos», written by
/// the runtime over whatever the model sent.
pub(crate) fn execute_tool(app: &AppHandle, context: &BackendRequestContext, name: &str, arguments: &Value) -> Result<Value, BackendError> {
    if !context.actor.is_library_owner() || !context.persistence_policy.allows_memory() {
        return Err(owner_only());
    }
    let library_id = context.library_id.as_str();
    if name == waits::WAIT_TOOL {
        return start_wait(app, context, arguments);
    }
    if name == "create_ai_task" {
        let approve_all = arguments.get("approveAll").and_then(Value::as_bool).unwrap_or(false);
        let task = create(app, library_id, context, tools::new_task(arguments)?, approve_all)?;
        return Ok(json!({
            "id": task.id,
            "title": task.title,
            "created": true,
            "approveAll": task.approve_all,
            "status": task.status.label(),
            "note": "La tarea quedó creada y el Planificador ya está armando el plan; si el objetivo pide esperar una hora o una respuesta, ese paso espera solo. La persona la sigue en Tareas IA. No la hagas vos en este turno. Para sumarle o corregirle algo al objetivo usá resume_ai_task con goal (en cualquier estado), sin crear otra tarea.",
        }));
    }
    let tasks = tasks_of(app, library_id)?;
    match name {
        "list_ai_tasks" => Ok(tools::list_view(&tasks)),
        "get_ai_task" => Ok(tools::detail_view(tools::resolve_task(&tasks, arguments)?)),
        "cancel_ai_task" => {
            let task = tools::resolve_task(&tasks, arguments)?;
            if cancel(app, library_id, &task.id)? {
                Ok(json!({ "id": task.id, "title": task.title, "cancelled": true, "status": AiTaskStatus::Cancelled.label() }))
            } else {
                Ok(json!({
                    "id": task.id,
                    "title": task.title,
                    "cancelled": true,
                    "note": "La tarea se está deteniendo: termina la llamada que tenía en curso y queda cancelada; hasta entonces list_ai_tasks puede mostrarla trabajando.",
                }))
            }
        }
        "resume_ai_task" => {
            let replan = tools::replan_request(arguments)?;
            let task = resume(app, library_id, &tools::resolve_task(&tasks, arguments)?.id, replan.as_ref())?;
            let (done, _) = task.progress();
            Ok(json!({
                "id": task.id,
                "title": task.title,
                "resumed": true,
                "goalChanged": replan.as_ref().is_some_and(|replan| replan.goal.is_some()),
                "restarted": replan.as_ref().is_some_and(|replan| replan.restart),
                "stepsKept": done,
                "status": task.status.label(),
            }))
        }
        "review_ai_task" => {
            let task = review_again(app, library_id, &tools::resolve_task(&tasks, arguments)?.id, tools::review_instructions(arguments))?;
            Ok(json!({ "id": task.id, "title": task.title, "reviewing": true }))
        }
        "delete_ai_task" => {
            let task = delete(app, library_id, &tools::resolve_task(&tasks, arguments)?.id)?;
            Ok(json!({ "id": task.id, "title": task.title, "deleted": true }))
        }
        _ => Err(BackendError::invalid_input("La herramienta no es de Tareas IA.")),
    }
}

// ---------------------------------------------------------------------------
// Waits
// ---------------------------------------------------------------------------

/// `wait_ai_task`: the executor's run that calls it is found by its
/// request; the wait is saved on the task and begins when the run ends.
fn start_wait(app: &AppHandle, context: &BackendRequestContext, arguments: &Value) -> Result<Value, BackendError> {
    let library_id = context.library_id.as_str();
    let run = runners().lock().ok().and_then(|runners| {
        runners.iter().find_map(|(task_id, runner)| {
            runner
                .current
                .as_ref()
                .filter(|run| run.context.request_id == context.request_id)
                .map(|run| (task_id.clone(), run.phase, run.step_id.clone(), run.started_ms))
        })
    });
    let Some((task_id, AiTaskPhase::Executor, Some(step_id), run_started_ms)) = run else {
        return Err(BackendError::invalid_input("wait_ai_task solo se usa en el Ejecutor de una Tarea IA, mientras hace un paso."));
    };
    let accounts = crate::mail_tools::gmail_accounts(app, library_id).into_iter().map(|account| account.email).collect::<Vec<_>>();
    let zone = library_zone(app, library_id);
    let now = now_ms();
    let mut wait = waits::wait_request(arguments, &step_id, &accounts, &zone, now)?;
    if let WaitFor::Mail { account, thread_id: Some(thread_id), .. } = &wait.wait_for {
        // A thread that does not exist would wait for nothing.
        mail_thread(app, library_id, account, thread_id)?;
    }
    if let WaitFor::WhatsApp { chat, .. } = &wait.wait_for {
        let (chat, name, since_ms) = crate::whatsapp::wait_target(app, library_id, chat, run_started_ms)?;
        watched_chats().lock().map(|mut chats| chats.insert(chat.clone())).ok();
        wait.wait_for = WaitFor::WhatsApp { chat, name, since_ms };
    }
    let database = database(app, library_id)?;
    let saved = write(app, &database, |connection| {
        let Some(mut task) = load_task(connection, &task_id)? else {
            return Ok(false);
        };
        task.wait = Some(wait.clone());
        update_task(connection, &task)
    })?;
    if !saved {
        return Err(not_found());
    }
    Ok(json!({
        "waiting": true,
        "reason": wait.reason,
        "kind": waits::kind_label(&wait, &zone, now),
        "until": waits::deadline_label(&wait, &zone, now),
        "note": "Listo: al terminar esta respuesta la tarea queda esperando. Terminá ahora con el informe de lo que hiciste hasta acá en este paso; Notia te vuelve a llamar en este mismo paso con lo que pase.",
    }))
}

fn gmail_account(app: &AppHandle, library_id: &str, email: &str) -> Result<crate::backend::mail_tools::MailAccountRef, BackendError> {
    crate::mail_tools::gmail_accounts(app, library_id)
        .into_iter()
        .find(|account| account.email.eq_ignore_ascii_case(email))
        .ok_or_else(|| BackendError::invalid_input(format!("La cuenta de Gmail {email} ya no está conectada en la biblioteca.")))
}

/// The messages of a Gmail thread, with their dates and labels.
fn mail_thread(app: &AppHandle, library_id: &str, email: &str, thread_id: &str) -> Result<Vec<Value>, BackendError> {
    // Getting the session may refresh the token, which blocks by itself.
    let session = crate::mail_tools::gmail_session(app, library_id, &gmail_account(app, library_id, email)?)?;
    let thread = crate::host::async_runtime::block_on(session.get(&crate::backend::mail_tools::thread_metadata_url(thread_id))).map_err(|error| {
        if error.code == BackendErrorCode::NotFound {
            BackendError::invalid_input(format!("No hay un hilo {thread_id} en {email}: usá el threadId que devolvió send_gmail_message o list_gmail_messages."))
        } else {
            error
        }
    })?;
    Ok(thread.get("messages").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// The new messages a mail wait was waiting for, as summaries.
fn new_mail(app: &AppHandle, library_id: &str, wait: &AiTaskWait) -> Result<Vec<Value>, BackendError> {
    let WaitFor::Mail { account, thread_id, query } = &wait.wait_for else {
        return Ok(Vec::new());
    };
    let messages = match (thread_id, query) {
        (Some(thread_id), _) => mail_thread(app, library_id, account, thread_id)?,
        (None, Some(query)) => {
            let session = crate::mail_tools::gmail_session(app, library_id, &gmail_account(app, library_id, account)?)?;
            let search = waits::mail_search(query, wait.started_at_ms);
            crate::host::async_runtime::block_on(async {
                let url = crate::backend::mail_tools::list_messages_url(&search, &[], waits::MAX_WAKE_MESSAGES as u64, None);
                let list = session.get(&url).await?;
                let mut messages = Vec::new();
                for id in list.get("messages").and_then(Value::as_array).into_iter().flatten().filter_map(|item| item.get("id").and_then(Value::as_str)) {
                    messages.push(session.get(&crate::backend::mail_tools::message_metadata_url(id)).await?);
                }
                Ok::<_, BackendError>(messages)
            })?
        }
        (None, None) => Vec::new(),
    };
    Ok(waits::new_messages(&messages, wait.started_at_ms).iter().map(|message| crate::backend::mail_tools::message_summary(message, &[])).collect())
}

/// What the watch decided for a waiting task.
enum WaitStep {
    Wake(String),
    /// Look again later; with why the look failed, if it did.
    Later { error: Option<String> },
}

fn wait_step(app: &AppHandle, library_id: &str, wait: &AiTaskWait, zone: &jiff::tz::TimeZone, now: i64) -> Option<WaitStep> {
    match waits::due(wait, zone, now) {
        Due::NotYet => None,
        Due::Wake(note) => Some(WaitStep::Wake(note)),
        Due::LookAtMail => {
            let WaitFor::Mail { account, .. } = &wait.wait_for else { return None };
            let looked = new_mail(app, library_id, wait);
            Some(match looked {
                Ok(messages) if !messages.is_empty() => WaitStep::Wake(waits::mail_note(account, &messages)),
                _ if now >= wait.deadline_ms => {
                    let mut wait = wait.clone();
                    if let Err(error) = &looked {
                        wait.last_error = Some(error.message.clone());
                    }
                    WaitStep::Wake(waits::deadline_note(&wait, zone, now))
                }
                Ok(_) => WaitStep::Later { error: None },
                Err(error) => {
                    log::warn!("[notia:ai-tasks] no se pudo mirar el correo de una espera: {:?}", error.code);
                    WaitStep::Later { error: Some(error.message) }
                }
            })
        }
        Due::LookAtWhatsApp => {
            let WaitFor::WhatsApp { chat, since_ms, .. } = &wait.wait_for else { return None };
            let looked = crate::whatsapp::wait_replies(app, library_id, chat, *since_ms, now);
            Some(match looked {
                Ok(Some(note)) => WaitStep::Wake(note),
                _ if now >= wait.deadline_ms => {
                    let mut wait = wait.clone();
                    if let Err(error) = &looked {
                        wait.last_error = Some(error.message.clone());
                    }
                    WaitStep::Wake(waits::deadline_note(&wait, zone, now))
                }
                Ok(None) => WaitStep::Later { error: None },
                Err(error) => WaitStep::Later { error: Some(error.message) },
            })
        }
    }
}

/// The chats a waiting task waits for a WhatsApp reply in.
fn watched_chats() -> &'static Mutex<HashSet<String>> {
    static CHATS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CHATS.get_or_init(Default::default)
}

/// The moments the watch was asked to look before its interval, and the
/// signal that wakes it.
fn looks() -> &'static (Mutex<Vec<Instant>>, Condvar) {
    static LOOKS: OnceLock<(Mutex<Vec<Instant>>, Condvar)> = OnceLock::new();
    LOOKS.get_or_init(Default::default)
}

fn look_at_waits_at(at: Instant) {
    let (moments, signal) = looks();
    if let Ok(mut moments) = moments.lock() {
        moments.push(at);
        signal.notify_all();
    }
}

/// Someone else wrote in a WhatsApp chat: when a task waits for a reply
/// there, the watch looks once they could have stopped writing.
pub(crate) fn whatsapp_message_arrived(chat: &str) {
    if watched_chats().lock().is_ok_and(|chats| chats.contains(chat)) {
        look_at_waits_at(Instant::now() + crate::backend::reply_wait::WHATSAPP_TASK_WAIT + LOOK_MARGIN);
    }
}

/// Sleeps until the watch's interval passes or a look it was asked for is
/// due; the looks that are due are taken.
fn wait_for_next_look() {
    let (moments, signal) = looks();
    let Ok(mut pending) = moments.lock() else {
        std::thread::sleep(WAIT_WATCH_INTERVAL);
        return;
    };
    let interval_end = Instant::now() + WAIT_WATCH_INTERVAL;
    loop {
        let now = Instant::now();
        if pending.iter().any(|at| *at <= now) {
            pending.retain(|at| *at > now);
            return;
        }
        if now >= interval_end {
            return;
        }
        let until = pending.iter().copied().min().unwrap_or(interval_end).min(interval_end);
        pending = match signal.wait_timeout(pending, until - now) {
            Ok((pending, _)) => pending,
            Err(_) => return,
        };
    }
}

/// Looks at the waiting tasks of the open library and starts again the
/// ones whose wait ended.
fn watch_waits(app: &AppHandle) -> Result<(), BackendError> {
    // Only the computer that runs the library's services works the tasks.
    if !crate::device_role::runs_services(app) {
        return Ok(());
    }
    let Some(library) = crate::library_catalog::selected_library(app) else {
        return Ok(());
    };
    let Ok(database) = database(app, &library.id) else {
        return Ok(());
    };
    let held = runners().lock().map(|runners| runners.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let tasks = read(app, &database, load_tasks)?;
    // The chats of the waits, also of a run that asks for one now.
    let chats = tasks.iter().filter(|task| task.status == AiTaskStatus::Waiting || held.contains(&task.id)).filter_map(|task| {
        match task.wait.as_ref().map(|wait| &wait.wait_for) {
            Some(WaitFor::WhatsApp { chat, .. }) => Some(chat.clone()),
            _ => None,
        }
    });
    if let Ok(mut watched) = watched_chats().lock() {
        *watched = chats.collect();
    }
    let waiting = tasks.into_iter().filter(|task| task.status == AiTaskStatus::Waiting && !held.contains(&task.id)).collect::<Vec<_>>();
    if waiting.is_empty() {
        return Ok(());
    }
    let zone = library_zone(app, &library.id);
    for task in waiting {
        let Some(wait) = task.wait.clone() else { continue };
        let now = now_ms();
        let Some(step) = wait_step(app, &library.id, &wait, &zone, now) else { continue };
        // The task may have changed while the mail was read: only the same
        // wait is touched.
        let woke = write(app, &database, |connection| {
            let Some(mut task) = load_task(connection, &task.id)? else {
                return Ok(false);
            };
            if task.status != AiTaskStatus::Waiting || task.wait.as_ref() != Some(&wait) {
                return Ok(false);
            }
            match &step {
                WaitStep::Wake(note) => {
                    waits::wake(&mut task, note);
                    task.updated_at_ms = now;
                    update_task(connection, &task)
                }
                WaitStep::Later { error } => {
                    if let Some(current) = task.wait.as_mut() {
                        current.wake_at_ms = (now + waits::poll_ms(&current.wait_for)).min(current.deadline_ms);
                        current.last_error = error.clone();
                    }
                    update_task(connection, &task).map(|_| false)
                }
            }
        })?;
        if woke {
            emit(app, &library.id);
            spawn(app, &library.id, &task.id);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Start and clean-up
// ---------------------------------------------------------------------------

/// The tasks that were working when Notia closed become interrupted; the
/// ones older than a week are deleted.
fn clean_up(app: &AppHandle, first: bool) -> Result<(), BackendError> {
    // Only the computer that runs the library's services works the tasks.
    if !crate::device_role::runs_services(app) {
        return Ok(());
    }
    let Some(library) = crate::library_catalog::selected_library(app) else {
        return Ok(());
    };
    let Ok(database) = database(app, &library.id) else {
        return Ok(());
    };
    let now = now_ms();
    let held = runners().lock().map(|runners| runners.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let changed = write(app, &database, |connection| {
        let mut changed = delete_expired(connection, now)? > 0;
        if first {
            // A waiting task keeps waiting: the watch goes on with it.
            let stopped = |task: &AiTask| task.status.is_active() && task.status != AiTaskStatus::Waiting && !held.contains(&task.id);
            for mut task in load_tasks(connection)?.into_iter().filter(stopped) {
                task.status = AiTaskStatus::Interrupted;
                task.pending = None;
                task.phase = None;
                task.error = Some("Notia se cerró mientras la tarea trabajaba.".to_string());
                task.updated_at_ms = now;
                if let Some(step) = task.steps.iter_mut().find(|step| step.status == StepStatus::Running) {
                    step.status = StepStatus::Failed;
                }
                changed |= update_task(connection, &task)?;
            }
        }
        Ok(changed)
    })?;
    if changed {
        emit(app, &library.id);
    }
    Ok(())
}

pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("ai-tasks")
        .setup(|app, _api| {
            let watcher = app.clone();
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-ai-tasks-cleanup".into()).spawn(move || {
                std::thread::sleep(FIRST_CLEANUP_DELAY);
                let mut first = true;
                loop {
                    if let Err(error) = clean_up(&app, first) {
                        log::error!("[notia:ai-tasks] no se pudieron limpiar las tareas: {:?}", error.code);
                    }
                    first = false;
                    std::thread::sleep(CLEANUP_INTERVAL);
                }
            });
            let app = watcher;
            let _ = std::thread::Builder::new().name("notia-ai-task-waits".into()).spawn(move || {
                // After the first clean-up, so an interrupted task is marked first.
                std::thread::sleep(FIRST_CLEANUP_DELAY + WAIT_WATCH_INTERVAL);
                loop {
                    if let Err(error) = watch_waits(&app) {
                        log::error!("[notia:ai-tasks] no se pudieron revisar las esperas: {:?}", error.code);
                    }
                    wait_for_next_look();
                }
            });
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_in_a_watched_chat_makes_the_watch_look_early() {
        let pending = || looks().0.lock().map(|moments| moments.len()).unwrap_or_default();
        let before = pending();
        whatsapp_message_arrived("5491100000001@s.whatsapp.net");
        assert_eq!(pending(), before, "nobody waits in that chat");
        watched_chats().lock().expect("chats").insert("5491100000002@s.whatsapp.net".into());
        whatsapp_message_arrived("5491100000002@s.whatsapp.net");
        assert_eq!(pending(), before + 1);
        look_at_waits_at(Instant::now());
        let started = Instant::now();
        wait_for_next_look();
        assert!(started.elapsed() < Duration::from_secs(5), "the due look ends the sleep");
        assert_eq!(pending(), before + 1, "the later look is kept");
    }

    fn memory() -> Connection {
        let connection = Connection::open_in_memory().expect("memory");
        crate::database::migrate(&connection).expect("schema");
        connection
    }

    fn task(id: &str, created_at_ms: i64) -> AiTask {
        AiTask::new(id.into(), "Ordenar", "Objetivo", AiTaskOrigin::App, None, false, created_at_ms)
    }

    #[test]
    fn tasks_survive_a_round_trip_and_expire_after_a_week() {
        let connection = memory();
        let now = 10 * RETENTION_MS;
        insert_task(&connection, &task("old", now - RETENTION_MS - 1)).expect("old");
        let mut recent = task("new", now - 1_000);
        insert_task(&connection, &recent).expect("new");
        recent.status = AiTaskStatus::Running;
        recent.set_plan(vec![crate::backend::ai_tasks::PlannedStep { title: "A".into(), detail: String::new() }]);
        assert!(update_task(&connection, &recent).expect("update"));
        assert_eq!(load_task(&connection, "new").expect("load"), Some(recent.clone()));
        // A task that waits is kept however old.
        let mut waiting = task("waiting", now - 3 * RETENTION_MS);
        waiting.status = AiTaskStatus::Waiting;
        insert_task(&connection, &waiting).expect("waiting");
        assert_eq!(delete_expired(&connection, now).expect("expire"), 1);
        connection.execute("DELETE FROM ai_tasks WHERE id='waiting'", []).expect("delete waiting");
        assert_eq!(load_tasks(&connection).expect("tasks").iter().map(|task| task.id.as_str()).collect::<Vec<_>>(), ["new"]);
        // A deleted task is not written again by its runner.
        connection.execute("DELETE FROM ai_tasks WHERE id='new'", []).expect("delete");
        assert!(!update_task(&connection, &recent).expect("gone"));
    }

    fn confirmation() -> (PendingInteraction, OperationToken) {
        let operation: OperationToken = serde_json::from_value(json!({ "operationId": "op-1", "generation": 1, "kind": "confirmation" }))
            .unwrap_or_else(|_| serde_json::from_value(json!({ "operationId": "op-1" })).expect("token"));
        let request: crate::backend::ConfirmationRequest = serde_json::from_value(json!({
            "operation": operation,
            "preview": { "operationId": "op-1", "summary": "Editar Indice.md", "documents": [], "hunks": [
                { "id": "h1", "documentPath": "Indice.md", "startLine": 1, "endLine": 1, "oldText": "", "newText": "nextPage: x" }
            ], "allowedActions": ["apply-all", "reject", "cancel"] }
        }))
        .expect("confirmation");
        (PendingInteraction::Confirmation(request), operation)
    }

    #[test]
    fn answers_become_the_decisions_the_run_resumes_with() {
        let (interaction, operation) = confirmation();
        let approval = approval_of(&interaction, AiTaskPhase::Executor, Some("s2".into()));
        assert_eq!((approval.kind, approval.title.as_str(), approval.detail.as_str()), (ApprovalKind::Confirmation, "Editar Indice.md", "nextPage: x"));
        assert_eq!(telegram_buttons(&approval).len(), 3);
        assert!(telegram_buttons(&approval)[1].1.starts_with("aitask:") && telegram_buttons(&approval)[1].1.ends_with(":a"));

        let all = ApprovalAnswer { choice: AnswerChoice::ConfirmAll, text: None, option_index: None };
        match decision_for(&interaction, &operation, &all).expect("decision").0 {
            ResumeDecision::Confirmation(decision) => assert!(decision.accepted && decision.approve_all),
            other => panic!("{other:?}"),
        }
        let propose = ApprovalAnswer { choice: AnswerChoice::Propose, text: Some("  ".into()), option_index: None };
        assert!(decision_for(&interaction, &operation, &propose).is_err());
        let wrong = ApprovalAnswer { choice: AnswerChoice::Answer, text: Some("sí".into()), option_index: None };
        assert!(decision_for(&interaction, &operation, &wrong).is_err());
    }
}
