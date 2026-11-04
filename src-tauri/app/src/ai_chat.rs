//! Turns of the app chats: the sidebar and main chat, questions about the
//! Meeting transcript and questions of a published Task Manager that the
//! host answers.
//!
//! A turn reads the chat, picks the messages the agent sees, runs the agent,
//! asks the interface for the clarifications, confirmations and plans the
//! agent needs, saves the turn and schedules the chat title. A chat with
//! agents runs them instead of Notia: each agent answers with the chat's
//! permissions, and they keep answering each other for a few rounds. The
//! interface sends the message and the visible workspace, renders the
//! streamed events and answers the questions it is asked. A message sent
//! while a turn runs goes through a short call to the model that decides
//! whether it stops the turn or waits for the next one; a turn stopped that
//! way is saved with a note, so the next message keeps its context.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::{Deserialize, Serialize};
use crate::host::{AppHandle, Emitter, Manager};

use crate::backend::ai_settings::AiSettingsInput;
use crate::backend::chat_history::{ChatAttachmentKind, ChatRole, StoredChatAttachment, StoredChatDocument, StoredChatMessage};
use crate::backend::chat_context::{self, ContextFile};
use crate::backend::chat_history::ChatContextMode;
use crate::backend::chat_agents::{self as agent_rules, ChatAgent};
use crate::backend::chat_turn::{self, ContextSelection, TurnMode, WorkspaceInput};
use crate::backend::{
    AgentRequest, AgentResponse, BackendActor, BackendError, BackendErrorCode, BackendEvent, BackendRequest,
    BackendRequestContext, BackendRequestEnvelope, BackendResponse, CancelRequest, ClarificationAnswer,
    ConfirmationDecision, ExecutionPlan, GetOperationRequest, MutationPreview, OperationToken, PendingInteraction, PlanDecision,
    PlanStepStatus, ProtocolVersion, ResumeDecision, ResumeRequest, UndoOperationRequest, OWNER_LIBRARY_USER_ID,
};
use crate::backend::turn_interrupts::{InterruptDecision, CANCELLED_REPLY};
use crate::backend_runtime::{execute_backend_request, BackendRuntimeState};
use crate::library_registry::LibraryBindingRegistry;

/// A question of the agent the interface must answer.
pub(crate) const INTERACTION_EVENT: &str = "ai-chat-interaction";
/// The AI title of a new chat was saved.
pub(crate) const TITLE_EVENT: &str = "ai-chat-title";
/// An agent of the chat starts answering, or finished its message.
pub(crate) const AGENT_EVENT: &str = "ai-chat-agent";
/// How long a question waits for the person before the turn is cancelled.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How often a published turn passes the runtime's new events on.
const RELAY_INTERVAL: Duration = Duration::from_millis(120);

/// Where the events of a turn go when it is not the window's: the published
/// Task Manager page that asked. It takes the event's name, as the window
/// would receive it, and its payload; it returns whether it was delivered.
pub(crate) type TurnForward = Arc<dyn Fn(&str, serde_json::Value) -> bool + Send + Sync>;
/// Changed operations of this session that the interface may undo.
const MAX_UNDOABLE_OPERATIONS: usize = 50;

#[derive(Debug, Clone)]
struct RequestIdentity {
    context: BackendRequestContext,
    idempotency_key: String,
}

struct ActiveTurn {
    identity: RequestIdentity,
    /// The person's message, for the decision about messages sent meanwhile.
    message: String,
    /// Present while a question waits for the person.
    answer: Option<mpsc::Sender<InteractionAnswer>>,
    cancelled: bool,
    /// A message sent during the turn asked to stop it.
    stopped_by_message: bool,
    /// Prompt file of the agent that is answering; `None` for Notia.
    speaker: Option<String>,
    /// Questions the agent asked and the person's answers, not yet placed
    /// among the turn's replies.
    exchanges: Vec<StoredChatMessage>,
    /// A published turn: its questions and messages go to the page.
    forward: Option<TurnForward>,
}

/// Sends a turn's event to the page that asked, or to the window.
fn deliver<T: Serialize + Clone>(app: &AppHandle, forward: Option<&TurnForward>, event: &str, payload: T) -> bool {
    match forward {
        Some(forward) => serde_json::to_value(payload).map(|value| forward(event, value)).unwrap_or(false),
        None => app.emit(event, payload).is_ok(),
    }
}

/// Passes the runtime events of a published turn to its page while it runs;
/// dropping it sends what is left and stops.
struct EventRelay {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EventRelay {
    fn start(runtime: BackendRuntimeState, context: crate::backend::BackendRequestContext, forward: TurnForward) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("notia-published-chat-events".into())
            .spawn(move || {
                let mut after = 0;
                loop {
                    let last_pass = stopping.load(Ordering::SeqCst);
                    for envelope in runtime.request_events(&context, after).unwrap_or_default() {
                        after = after.max(envelope.sequence);
                        if let Ok(value) = serde_json::to_value(&envelope) {
                            forward(crate::backend_tauri::BACKEND_EVENT, value);
                        }
                    }
                    if last_pass {
                        break;
                    }
                    std::thread::sleep(RELAY_INTERVAL);
                }
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for EventRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(crate) struct AiChatState {
    turns: Mutex<HashMap<String, ActiveTurn>>,
    undoable: Mutex<VecDeque<(String, RequestIdentity)>>,
}

/// Chat the turn belongs to.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub(crate) enum ChatTarget {
    /// A chat file of the library, by the path the interface holds.
    Saved { path: String },
    /// Previous messages of a conversation without settings (Meeting, published boards).
    Transient {
        #[serde(default)]
        messages: Vec<StoredChatMessage>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatSendPayload {
    library_id: String,
    /// Chosen by the interface to follow the turn's events.
    request_id: String,
    mode: TurnMode,
    /// Provider preferences in use, for a library without saved AI settings.
    #[serde(default)]
    settings: Option<AiSettingsInput>,
    /// Scope of the chat (`library`, `document`, `task-manager`, `graph`, `finance`).
    #[serde(default)]
    scope: String,
    message: String,
    /// Temporary context of the turn: the active room or view, or the Meeting transcript.
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    attachments: Vec<StoredChatAttachment>,
    /// Custom prompt file under `.agent/promps`.
    #[serde(default)]
    prompt_name: Option<String>,
    /// The person asked to undo this AI change.
    #[serde(default)]
    undo_operation_id: Option<String>,
    #[serde(default)]
    workspace: Option<WorkspaceInput>,
    #[serde(default)]
    selection: ContextSelection,
    /// Library user of a published Task Manager asking through the host.
    #[serde(default)]
    library_user_id: Option<String>,
    chat: ChatTarget,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatTurnOutcome {
    /// Notia's answer, or the last agent's message.
    answer: String,
    /// The agent changed library data (the open note may need a reload).
    data_changed: bool,
    /// The chat after the turn, when the turn belongs to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    document: Option<StoredChatDocument>,
    /// The AI change this turn undid.
    #[serde(skip_serializing_if = "Option::is_none")]
    undone_operation_id: Option<String>,
    /// Why the turn stopped; what happened until then is saved in `document`.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BackendError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanStepDto {
    id: String,
    label: String,
    status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
enum InteractionDto {
    Clarification { question: String, choices: Vec<String> },
    Confirmation { preview: MutationPreview },
    Plan { title: String, steps: Vec<PlanStepDto> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InteractionEvent {
    request_id: String,
    interaction: InteractionDto,
}

/// The person's answer to a question of the agent.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub(crate) enum InteractionAnswer {
    Clarification { answer: String },
    Confirmation {
        accepted: bool,
        #[serde(default)]
        hunk_ids: Vec<String>,
        /// «Confirmar todos»: the rest of the turn asks no more.
        #[serde(default)]
        approve_all: bool,
        /// «Proponer otra cosa»: what to do instead of the change.
        #[serde(default)]
        suggestion: Option<String>,
    },
    Plan {
        accepted: bool,
        /// «Confirmar todos»: the plan and the rest of the turn's changes.
        #[serde(default)]
        approve_all: bool,
        /// Changes the person asked for instead of approving.
        #[serde(default)]
        suggestion: Option<String>,
        /// Steps kept by the person; all of them when absent.
        #[serde(default)]
        step_ids: Option<Vec<String>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatAnswerPayload {
    request_id: String,
    answer: InteractionAnswer,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatCancelPayload {
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatInterjectPayload {
    /// The turn that runs.
    request_id: String,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatInterjectOutcome {
    /// `cancel`, `cancel-and-queue` or `queue`: the interface sends the
    /// message after the turn unless it only asked to stop it.
    decision: InterruptDecision,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSpeakerDto {
    file_name: String,
    name: String,
    initials: String,
}

/// Progress of the agents of a turn. `runRequestId` identifies the engine
/// run of the agent, whose streamed events carry that id.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "kebab-case", rename_all_fields = "camelCase")]
enum AgentPhase {
    Start { run_request_id: String, agent: AgentSpeakerDto },
    Message { run_request_id: String, message: StoredChatMessage },
    /// The agent answered nothing.
    Silent { run_request_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentEvent {
    request_id: String,
    #[serde(flatten)]
    phase: AgentPhase,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleEvent {
    library_id: String,
    path: String,
    title: String,
}

fn internal(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Internal, message, true)
}

fn cancelled() -> BackendError {
    BackendError::new(BackendErrorCode::Cancelled, "Consulta cancelada.", false)
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or_default()
}

fn envelope(request: BackendRequest) -> BackendRequestEnvelope {
    BackendRequestEnvelope { protocol_version: ProtocolVersion::default(), request }
}

fn execute(app: &AppHandle, runtime: &BackendRuntimeState, request: BackendRequest) -> Result<BackendResponse, BackendError> {
    let registry = app.state::<LibraryBindingRegistry>();
    Ok(execute_backend_request(app, envelope(request), runtime, registry.inner())?.response)
}

fn interaction_dto(interaction: &PendingInteraction) -> InteractionDto {
    match interaction {
        PendingInteraction::Clarification(request) => InteractionDto::Clarification {
            question: request.question.clone(),
            choices: request.options.iter().map(|option| option.label.clone()).collect(),
        },
        PendingInteraction::Confirmation(request) => InteractionDto::Confirmation { preview: request.preview.clone() },
        PendingInteraction::Plan(plan) => InteractionDto::Plan {
            title: plan.title.clone(),
            steps: plan
                .steps
                .iter()
                .map(|step| PlanStepDto {
                    id: step.id.clone(),
                    label: step.label.clone(),
                    status: match step.status {
                        PlanStepStatus::Completed => "completed",
                        PlanStepStatus::Blocked => "blocked",
                        _ => "pending",
                    },
                })
                .collect(),
        },
    }
}

/// The decision the runtime resumes with; `None` when the answer does not
/// match the question.
fn decision(interaction: &PendingInteraction, operation: &OperationToken, answer: InteractionAnswer) -> Option<ResumeDecision> {
    match (interaction, answer) {
        (PendingInteraction::Clarification(request), InteractionAnswer::Clarification { answer }) => {
            let option_id = request.options.iter().find(|option| option.label == answer).map(|option| option.id.clone());
            Some(ResumeDecision::Clarification(ClarificationAnswer {
                clarification_id: request.clarification_id.clone(),
                operation: operation.clone(),
                answer,
                option_id,
            }))
        }
        (PendingInteraction::Confirmation(_), InteractionAnswer::Confirmation { accepted, hunk_ids, approve_all, suggestion }) => {
            let suggestion = suggestion.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
            Some(ResumeDecision::Confirmation(ConfirmationDecision {
                operation_id: operation.operation_id.clone(),
                accepted: accepted && suggestion.is_none(),
                // A proposal rejects the change as a whole.
                hunk_ids: if suggestion.is_some() { Vec::new() } else { hunk_ids },
                approve_all: accepted && approve_all,
                suggestion,
            }))
        }
        (PendingInteraction::Plan(plan), InteractionAnswer::Plan { accepted, approve_all, step_ids, suggestion }) => Some(ResumeDecision::Plan(PlanDecision {
            plan_id: plan.plan_id.clone(),
            generation: plan.generation,
            accepted,
            step_ids: step_ids.unwrap_or_else(|| plan.steps.iter().map(|step| step.id.clone()).collect()),
            suggestion: suggestion.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
            approve_all: accepted && approve_all,
        })),
        _ => None,
    }
}

/// Shows a question of the agent and waits for the person's answer.
fn ask(
    app: &AppHandle,
    state: &AiChatState,
    request_id: &str,
    interaction: &PendingInteraction,
    operation: &OperationToken,
) -> Option<ResumeDecision> {
    let (sender, receiver) = mpsc::channel();
    let forward = {
        let mut turns = state.turns.lock().ok()?;
        let turn = turns.get_mut(request_id)?;
        if turn.cancelled {
            return None;
        }
        turn.answer = Some(sender);
        turn.forward.clone()
    };
    let event = InteractionEvent { request_id: request_id.to_string(), interaction: interaction_dto(interaction) };
    let answer = if deliver(app, forward.as_ref(), INTERACTION_EVENT, event) { receiver.recv_timeout(ANSWER_TIMEOUT).ok() } else { None };
    if let Ok(mut turns) = state.turns.lock() {
        if let Some(turn) = turns.get_mut(request_id) {
            turn.answer = None;
        }
    }
    let answer = answer?;
    match (interaction, &answer) {
        (PendingInteraction::Clarification(request), InteractionAnswer::Clarification { answer }) => {
            record_exchange(app, state, request_id, &request.question, answer);
        }
        (PendingInteraction::Plan(plan), InteractionAnswer::Plan { accepted, approve_all, step_ids, suggestion }) => {
            let answer = if *accepted && *approve_all {
                "Aprobado, junto con todo lo que siga en este pedido.".to_string()
            } else {
                plan_answer(*accepted, suggestion.as_deref())
            };
            record_exchange(app, state, request_id, &plan_text(plan, step_ids.as_deref()), &answer);
        }
        // A proposal instead of a change stays in the chat, like an answer.
        (PendingInteraction::Confirmation(request), InteractionAnswer::Confirmation { suggestion: Some(suggestion), .. })
            if !suggestion.trim().is_empty() =>
        {
            record_exchange(app, state, request_id, &request.preview.summary, suggestion.trim());
        }
        _ => {}
    }
    decision(interaction, operation, answer)
}

/// The plan as the chat keeps it: its title and the steps the person kept.
fn plan_text(plan: &ExecutionPlan, step_ids: Option<&[String]>) -> String {
    let steps = plan
        .steps
        .iter()
        .filter(|step| step_ids.is_none_or(|kept| kept.contains(&step.id)))
        .enumerate()
        .map(|(index, step)| format!("{}. {}", index + 1, step.label))
        .collect::<Vec<_>>()
        .join("\n");
    format!("**{}**\n\n{steps}", plan.title)
}

/// What the person answered to a plan, as the chat keeps it.
fn plan_answer(accepted: bool, suggestion: Option<&str>) -> String {
    match (accepted, suggestion.map(str::trim).filter(|value| !value.is_empty())) {
        (true, _) => "Aprobado.".to_string(),
        (false, Some(suggestion)) => suggestion.to_string(),
        (false, None) => "Cancelado.".to_string(),
    }
}

/// Keeps a question the agent asked and the person's answer: the thread
/// shows them at once and the turn saves them before the agent's reply, so
/// they neither vanish from the screen nor from the chat's history.
fn record_exchange(app: &AppHandle, state: &AiChatState, request_id: &str, question: &str, answer: &str) {
    let (run_request_id, messages, forward) = {
        let Ok(mut turns) = state.turns.lock() else { return };
        let Some(turn) = turns.get_mut(request_id) else { return };
        let messages = exchange_messages(question, answer, turn.speaker.clone());
        turn.exchanges.extend(messages.iter().cloned());
        (turn.identity.context.request_id.clone(), messages, turn.forward.clone())
    };
    for message in messages {
        let phase = AgentPhase::Message { run_request_id: run_request_id.clone(), message };
        deliver(app, forward.as_ref(), AGENT_EVENT, AgentEvent { request_id: request_id.to_string(), phase });
    }
}

fn exchange_messages(question: &str, answer: &str, speaker: Option<String>) -> [StoredChatMessage; 2] {
    [
        StoredChatMessage { role: ChatRole::Assistant, content: question.trim().to_string(), attachments: Vec::new(), agent: speaker },
        StoredChatMessage { role: ChatRole::User, content: answer.trim().to_string(), attachments: Vec::new(), agent: None },
    ]
}

/// Keeps, in order, what the agent wrote while it worked (the notes that
/// come with its tool calls) and the commands it ran in the terminal, with
/// their output. The thread showed them live; the turn saves them among the
/// questions and before the answer, so they stay.
fn keep_notes(state: &AiChatState, runtime: &BackendRuntimeState, turn_id: &str, identity: &RequestIdentity, seen: &mut u64) {
    let Ok(events) = runtime.request_events(&identity.context, *seen) else { return };
    let mut notes = Vec::new();
    for envelope in events {
        *seen = (*seen).max(envelope.sequence);
        let text = match envelope.event {
            BackendEvent::AssistantNote { text, .. } | BackendEvent::TerminalFinished { markdown: text, .. } => text,
            _ => continue,
        };
        let text = text.trim();
        if !text.is_empty() {
            notes.push(text.to_string());
        }
    }
    if notes.is_empty() {
        return;
    }
    let Ok(mut turns) = state.turns.lock() else { return };
    let Some(turn) = turns.get_mut(turn_id) else { return };
    let speaker = turn.speaker.clone();
    turn.exchanges.extend(notes.into_iter().map(|content| StoredChatMessage {
        role: ChatRole::Assistant,
        content,
        attachments: Vec::new(),
        agent: speaker.clone(),
    }));
}

/// The exchanges of the turn not placed yet among its replies.
fn take_exchanges(state: &AiChatState, request_id: &str) -> Vec<StoredChatMessage> {
    state
        .turns
        .lock()
        .ok()
        .and_then(|mut turns| turns.get_mut(request_id).map(|turn| std::mem::take(&mut turn.exchanges)))
        .unwrap_or_default()
}

/// Replies of a turn one agent answered: the questions answered on the way,
/// then its answer. A saved chat keeps a turn that failed (see
/// [`keep_failed_turn`]); other chats only report the error.
fn single_agent_replies(
    exchanges: Vec<StoredChatMessage>,
    response: Result<(StoredChatMessage, bool), BackendError>,
    worked: bool,
    keeps_failures: bool,
) -> Result<TurnReplies, BackendError> {
    match response {
        Ok((reply, changed)) => {
            let mut replies = exchanges;
            replies.push(reply);
            Ok((replies, changed, None))
        }
        Err(error) if keeps_failures => keep_failed_turn(exchanges, worked, error),
        Err(error) => Err(error),
    }
}

/// A turn that fails after the agent worked (it ran tools, answered or
/// asked something) stays in the chat: the person's message, what was said
/// and a note with the reason, instead of disappearing. One that failed
/// before doing anything is not saved, so the message goes back to the
/// composer to send again. A cancel keeps what was said, without a note.
fn keep_failed_turn(mut replies: Vec<StoredChatMessage>, worked: bool, error: BackendError) -> Result<TurnReplies, BackendError> {
    if error.code == BackendErrorCode::Cancelled {
        return if replies.is_empty() { Err(error) } else { Ok((replies, true, Some(error))) };
    }
    if replies.is_empty() && !worked {
        return Err(error);
    }
    replies.push(notia_reply(format!("No pude terminar: {}", error.message)));
    Ok((replies, true, Some(error)))
}

fn cancel_request(app: &AppHandle, runtime: &BackendRuntimeState, identity: &RequestIdentity, operation: Option<OperationToken>) {
    let _ = execute(
        app,
        runtime,
        BackendRequest::Cancel(CancelRequest {
            context: identity.context.clone(),
            idempotency_key: identity.idempotency_key.clone(),
            request_id: identity.context.request_id.clone(),
            operation,
        }),
    );
}

/// Runs the agent until it answers, resuming after each question, with no
/// limit: every question waits for the person, who can cancel. The questions
/// go to the turn `turn_id`, whose run may be one of its agents.
fn run_agent(
    app: &AppHandle,
    state: &AiChatState,
    runtime: &BackendRuntimeState,
    turn_id: &str,
    identity: &RequestIdentity,
    mut request: BackendRequest,
) -> Result<AgentResponse, BackendError> {
    let mut seen_events = 0;
    loop {
        let response = execute(app, runtime, request);
        keep_notes(state, runtime, turn_id, identity, &mut seen_events);
        if let Err(error) | Ok(BackendResponse::Error { error, .. }) = &response {
            if error.code != BackendErrorCode::Cancelled {
                crate::backend_runtime::log_request_failure(runtime, "chat", &identity.context, error);
            }
        }
        match response? {
            BackendResponse::Result { response } => return Ok(response),
            BackendResponse::Error { error, .. } => return Err(error),
            BackendResponse::Cancelled { .. } => return Err(cancelled()),
            BackendResponse::Operation { status } | BackendResponse::Resumed { status, .. } => {
                if let Some(response) = status.response {
                    return Ok(response);
                }
                let (Some(operation), Some(interaction)) = (status.operation, status.interaction) else {
                    break;
                };
                let Some(decision) = ask(app, state, turn_id, &interaction, &operation) else {
                    cancel_request(app, runtime, identity, Some(operation));
                    return Err(cancelled());
                };
                request = BackendRequest::Resume(ResumeRequest {
                    context: identity.context.clone(),
                    idempotency_key: identity.idempotency_key.clone(),
                    request_id: identity.context.request_id.clone(),
                    operation,
                    last_event_sequence: status.last_event_sequence,
                    decision,
                });
            }
            _ => break,
        }
    }
    let error = internal("El runtime backend no devolvió una respuesta final.");
    crate::backend_runtime::log_request_failure(runtime, "chat", &identity.context, &error);
    Err(error)
}

/// Remembers the changes of a finished request so a later turn can undo
/// them. Returns whether the agent ran any tool.
fn remember_undoable(state: &AiChatState, runtime: &BackendRuntimeState, identity: &RequestIdentity) -> bool {
    let Ok(events) = runtime.request_events(&identity.context, 0) else {
        return false;
    };
    let worked = events.iter().any(|event| matches!(event.event, BackendEvent::ToolCompleted { .. }));
    let Ok(mut undoable) = state.undoable.lock() else {
        return worked;
    };
    for event in events {
        if let BackendEvent::ToolCompleted { ok: true, changed: Some(true), operation_id: Some(operation_id), .. } = event.event {
            undoable.retain(|(known, _)| known != &operation_id);
            undoable.push_back((operation_id, identity.clone()));
        }
    }
    while undoable.len() > MAX_UNDOABLE_OPERATIONS {
        undoable.pop_front();
    }
    worked
}

/// Undoes an AI change of this session. The runtime re-reads the stored
/// operation, checks the document revision and restores the previous state.
/// Returns the path of the restored document.
fn undo(app: &AppHandle, state: &AiChatState, runtime: &BackendRuntimeState, operation_id: &str) -> Result<Option<String>, BackendError> {
    let identity = state
        .undoable
        .lock()
        .map_err(|_| internal("No se pudo deshacer el cambio."))?
        .iter()
        .find(|(known, _)| known == operation_id)
        .map(|(_, identity)| identity.clone())
        .ok_or_else(|| BackendError::invalid_input("La operación ya no está disponible para deshacer en esta sesión."))?;
    let status = execute(
        app,
        runtime,
        BackendRequest::GetOperation(GetOperationRequest {
            context: identity.context.clone(),
            idempotency_key: identity.idempotency_key.clone(),
            request_id: identity.context.request_id.clone(),
            operation: None,
            last_event_sequence: 0,
        }),
    )?;
    let operation = match status {
        BackendResponse::Operation { status } => status.operation,
        _ => None,
    }
    .filter(|operation| operation.operation_id == operation_id)
    .ok_or_else(|| BackendError::invalid_input("La operación ya no es la última de su solicitud y no puede deshacerse."))?;
    let response = execute(
        app,
        runtime,
        BackendRequest::Undo(UndoOperationRequest {
            context: identity.context.clone(),
            idempotency_key: identity.idempotency_key.clone(),
            request_id: identity.context.request_id.clone(),
            operation,
        }),
    )?;
    match response {
        BackendResponse::Undo { result } => {
            if let Ok(mut undoable) = state.undoable.lock() {
                undoable.retain(|(known, _)| known != operation_id);
            }
            if let Some(error) = result.result.error {
                return Err(error);
            }
            Ok(result
                .result
                .data
                .as_ref()
                .and_then(|data| data.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string))
        }
        BackendResponse::Error { error, .. } => Err(error),
        _ => Err(internal("No se pudo deshacer el cambio.")),
    }
}

/// Workspace of a turn: the one the interface sent, or the fixed view of a
/// Meeting or published question.
fn workspace_of(mode: TurnMode, workspace: Option<WorkspaceInput>) -> Result<WorkspaceInput, BackendError> {
    let fixed = |view: &str, scope: &str| WorkspaceInput {
        snapshot_version: 1,
        view: view.to_string(),
        scope: scope.to_string(),
        active_document: None,
        open_tabs: Vec::new(),
        selection: None,
        captured_at: now_millis(),
    };
    match (mode, workspace) {
        (TurnMode::Chat, Some(workspace)) => Ok(workspace),
        (TurnMode::Chat, None) => Err(BackendError::invalid_input(
            "La solicitud de IA necesita una biblioteca activa y un contexto válido para ejecutarse en el backend.",
        )),
        (TurnMode::Meeting, _) => Ok(fixed("meeting", "library")),
        (TurnMode::Published, _) => Ok(fixed("task-manager", "published")),
    }
}

/// What a library chat turn adds for the library: the chosen files as a
/// context block and whether the agent may search the rest.
struct LibraryTurnContext {
    block: Option<String>,
    library_search: bool,
}

/// The chosen files and folders of the main library chat. Other chats keep
/// their own context (views, rooms, boards) and the full catalog.
fn library_context(app: &AppHandle, library_id: &str, mode: TurnMode, scope: &str, selection: &ContextSelection) -> LibraryTurnContext {
    if mode != TurnMode::Chat || scope != "library" || selection.keep_chat_context {
        return LibraryTurnContext { block: None, library_search: true };
    }
    let logical = |paths: &[String]| {
        paths
            .iter()
            .filter_map(|path| crate::library_session::resolve_logical_path(app, library_id, path).ok())
            .collect::<Vec<_>>()
    };
    let (files, folders) = (logical(&selection.files), logical(&selection.folders));
    let inventory = if folders.is_empty() {
        Vec::new()
    } else {
        crate::library_inventory::inventory_files(app, library_id).map(|(paths, _)| paths).unwrap_or_default()
    };
    let files = chat_context::expand_context_files(&files, &folders, &inventory)
        .into_iter()
        .map(|path| {
            let content = (selection.mode == ChatContextMode::Direct)
                .then(|| crate::library_session::read_library_text(app, library_id, &path).ok())
                .flatten();
            ContextFile { path, content }
        })
        .collect::<Vec<_>>();
    LibraryTurnContext {
        block: chat_context::context_block(selection.mode, selection.library_rag, &files),
        library_search: selection.library_rag,
    }
}

/// Names a new chat after its first turn, without delaying the answer.
fn schedule_chat_title(app: &AppHandle, library_id: &str, logical_path: String, prompt: String) {
    let app = app.clone();
    let library_id = library_id.to_string();
    std::thread::spawn(move || match crate::agent_knowledge::title_chat(&app, &library_id, &logical_path, &prompt) {
        Ok(Some(title)) => {
            let path = crate::library_session::visible_path(&app, &library_id, &logical_path);
            let _ = app.emit(TITLE_EVENT, TitleEvent { library_id: library_id.clone(), path, title });
        }
        Ok(None) => {}
        Err(error) => log::warn!("[notia:chat] no se pudo titular el chat: {}", error.message),
    });
}

/// What every run of a turn shares: the chat's library context, its tools
/// and its permanent context.
struct TurnSetup {
    library_id: String,
    snapshot: crate::backend::BackendSnapshot,
    library_block: Option<String>,
    library_search: bool,
    tool_access: crate::backend::ToolAccess,
    permanent_block: Option<String>,
}

impl TurnSetup {
    /// The prompt of a run with the chat's permanent and library context.
    fn prompt(&self, prompt: String) -> String {
        let prompt = match &self.permanent_block {
            Some(block) => format!("{prompt}\n\n{block}"),
            None => prompt,
        };
        chat_context::prompt_with_context(prompt, self.library_block.clone())
    }

    fn request(
        &self,
        identity: &RequestIdentity,
        history: &[StoredChatMessage],
        prompt: &str,
        attachments: &[StoredChatAttachment],
        prompt_name: Option<String>,
    ) -> BackendRequest {
        BackendRequest::Run(AgentRequest {
            context: identity.context.clone(),
            messages: chat_turn::turn_messages(history, prompt, attachments),
            snapshot: Some(self.snapshot.clone()),
            tools: Vec::new(),
            attachments: Vec::new(),
            idempotency_key: identity.idempotency_key.clone(),
            prompt_name,
            tool_access: self.tool_access,
            library_search: self.library_search,
            autonomous: false,
            scheduled_action: None,
            webhook_call: None,
            tool_areas: None,
            thinking: None,
            conversational: false,
            ai_task: None,
        })
    }
}

/// Messages the agents added in a turn and whether they changed data. When
/// a run fails, what the agents said before stays with the error.
struct AgentRounds {
    replies: Vec<StoredChatMessage>,
    changed: bool,
    /// Some agent ran a tool.
    worked: bool,
    error: Option<BackendError>,
}

fn is_cancelled(state: &AiChatState, turn_id: &str) -> bool {
    state.turns.lock().map(|turns| turns.get(turn_id).is_none_or(|turn| turn.cancelled)).unwrap_or(true)
}

/// The agents answer the person's message and then each other, for as many
/// rounds as the chain allows. Each agent runs as the engine with its own
/// prompt and the chat's permissions; cancelling stops the running agent.
#[allow(clippy::too_many_arguments)]
fn run_agent_rounds(
    app: &AppHandle,
    state: &AiChatState,
    runtime: &BackendRuntimeState,
    turn: &RequestIdentity,
    setup: &TurnSetup,
    agents: &[ChatAgent],
    dynamic: Option<&str>,
    conversation: &mut Vec<StoredChatMessage>,
) -> AgentRounds {
    let turn_id = turn.context.request_id.clone();
    let names = |file: &str| {
        agents
            .iter()
            .find(|agent| agent.file_name == file)
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| agent_rules::file_stem(file))
    };
    let mut random = || rand::thread_rng().gen::<f64>();
    let round_limit = agent_rules::automatic_round_limit(&mut random);
    let mut outcome = AgentRounds { replies: Vec::new(), changed: false, worked: false, error: None };
    let mut run = 0;
    for _ in 0..round_limit {
        let mut answered = 0;
        for index in agent_rules::select_participants(agents, dynamic, &mut random) {
            if is_cancelled(state, &turn_id) {
                outcome.error = Some(cancelled());
                return outcome;
            }
            let agent = &agents[index];
            run += 1;
            let run_id = format!("{turn_id}-{run}");
            let identity = RequestIdentity {
                idempotency_key: format!("{}:{run_id}", setup.library_id),
                context: BackendRequestContext { request_id: run_id.clone(), ..turn.context.clone() },
            };
            // Cancelling the turn now cancels this agent's run.
            if let Ok(mut turns) = state.turns.lock() {
                if let Some(active) = turns.get_mut(&turn_id) {
                    active.identity = identity.clone();
                    active.speaker = Some(agent.file_name.clone());
                }
            }
            let emit = |phase: AgentPhase| {
                let _ = app.emit(AGENT_EVENT, AgentEvent { request_id: turn_id.clone(), phase });
            };
            emit(AgentPhase::Start {
                run_request_id: run_id.clone(),
                agent: AgentSpeakerDto {
                    file_name: agent.file_name.clone(),
                    name: agent.name.clone(),
                    initials: agent_rules::initials(&agent.name),
                },
            });
            let others = agents
                .iter()
                .filter(|other| other.file_name != agent.file_name)
                .map(|other| other.name.clone())
                .collect::<Vec<_>>();
            let prompt = setup.prompt(agent_rules::agent_turn_prompt(agent, &others, dynamic));
            let history = agent_rules::agent_history(conversation, &names);
            let request = setup.request(&identity, &history, &prompt, &[], Some(agent.file_name.clone()));
            let response = run_agent(app, state, runtime, &turn_id, &identity, request);
            outcome.worked |= remember_undoable(state, runtime, &identity);
            // The questions this agent asked go before its answer, or stay
            // with the error.
            let exchanges = take_exchanges(state, &turn_id);
            conversation.extend(exchanges.iter().cloned());
            outcome.replies.extend(exchanges);
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    outcome.error = Some(error);
                    return outcome;
                }
            };
            outcome.changed |= response.changed || response.tool_results.iter().any(|result| result.ok && result.changed);
            let content = strip_speaker_name(response.response.markdown.trim(), &agent.name);
            let images = generated_attachments(&identity.context.request_id);
            if content.is_empty() && images.is_empty() {
                emit(AgentPhase::Silent { run_request_id: run_id });
                continue;
            }
            let message = StoredChatMessage {
                role: ChatRole::Assistant,
                content,
                attachments: images,
                agent: Some(agent.file_name.clone()),
            };
            conversation.push(message.clone());
            outcome.replies.push(message.clone());
            emit(AgentPhase::Message { run_request_id: run_id, message });
            answered += 1;
        }
        // A round without answers, or a dynamic that waits for the person,
        // ends the chain.
        if answered == 0 || !agent_rules::dynamic_allows_automatic_turns(dynamic) {
            break;
        }
    }
    outcome
}

/// Agents often start with their own name, as the history shows them.
fn strip_speaker_name(content: &str, name: &str) -> String {
    content
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix(':'))
        .map_or(content, str::trim_start)
        .to_string()
}

fn notia_reply(content: String) -> StoredChatMessage {
    StoredChatMessage { role: ChatRole::Assistant, content, attachments: Vec::new(), agent: None }
}

/// The reply to a `/image` message: the image (an edit of the first image
/// the person attached, or a new one) with a short text, or why it could
/// not be made. Cancelling the turn stops it.
fn image_command_reply(
    app: &AppHandle,
    library_id: &str,
    request_id: &str,
    command: Result<notia_backend_core::image_generation::ImageCommand, BackendError>,
    attachments: &[StoredChatAttachment],
) -> StoredChatMessage {
    let command = match command {
        Ok(command) => command,
        Err(error) => return notia_reply(error.message),
    };
    let reference = attachments
        .iter()
        .find(|attachment| attachment.kind == ChatAttachmentKind::Image && !attachment.base64.is_empty())
        .map(|attachment| (attachment.name.clone(), attachment.base64.clone()));
    let control = notia_backend_core::RequestControl::new(None);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (watched_app, turn_id) = (app.clone(), request_id.to_string());
    crate::image_generation::cancel_when(&control, std::sync::Arc::clone(&done), move || {
        is_cancelled(&watched_app.state::<AiChatState>(), &turn_id)
    });
    let result = crate::image_generation::run_command(app, library_id, &control, request_id, &command, reference);
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut reply = notia_reply(match result {
        Ok(text) => text,
        Err(error) => error.message,
    });
    reply.attachments = generated_attachments(request_id);
    reply
}

/// The images the agent made in `request_id`, as attachments of its reply:
/// saved with it in the chat file, so the chat shows them and a later turn
/// can edit them by their reference.
fn generated_attachments(request_id: &str) -> Vec<StoredChatAttachment> {
    crate::image_generation::take_generated(request_id)
        .into_iter()
        .map(|image| StoredChatAttachment {
            name: image.name,
            mime_type: image.media_type.to_string(),
            base64: image.base64,
            additional_base64: Vec::new(),
            kind: ChatAttachmentKind::Image,
            extracted_text: None,
            text_content: None,
            page_count: None,
        })
        .collect()
}

/// The prompt file that answered a turn alone signs its reply, so the chat
/// history shows which agent a chat talks with; the default prompt is Notia.
fn prompt_agent(prompt_name: Option<&str>) -> Option<String> {
    prompt_name
        .map(str::trim)
        .filter(|name| {
            notia_backend_core::chat_agents::is_valid_markdown_file_name(name)
                && !name.eq_ignore_ascii_case(notia_backend_core::agent_workspace::DEFAULT_PROMPT_FILE)
        })
        .map(str::to_string)
}

/// Replies of a turn, whether they changed data and the error that stopped
/// the agents after some of them answered.
type TurnReplies = (Vec<StoredChatMessage>, bool, Option<BackendError>);

fn send(app: &AppHandle, payload: ChatSendPayload) -> Result<ChatTurnOutcome, BackendError> {
    send_with(app, payload, None)
}

/// A question of a published Task Manager: what its guest wrote and the
/// conversation so far, which only the page keeps.
// The publication server, which asks for these turns, runs on Windows.
#[cfg(target_os = "windows")]
pub(crate) struct PublishedTurn {
    pub library_id: String,
    pub library_user_id: String,
    pub request_id: String,
    pub message: String,
    pub messages: Vec<StoredChatMessage>,
    pub attachments: Vec<StoredChatAttachment>,
}

/// Runs a published turn: the Task Manager agent without memory, over the
/// published boards, as the session's library user. Its events, questions
/// and messages go to `forward`; nothing is saved.
// The publication server, which asks for these turns, runs on Windows.
#[cfg(target_os = "windows")]
pub(crate) fn run_published_turn(app: &AppHandle, turn: PublishedTurn, forward: TurnForward) -> Result<ChatTurnOutcome, BackendError> {
    let payload = ChatSendPayload {
        library_id: turn.library_id,
        request_id: turn.request_id,
        mode: TurnMode::Published,
        settings: None,
        scope: "task-manager".to_string(),
        message: turn.message,
        context: None,
        attachments: turn.attachments,
        prompt_name: None,
        undo_operation_id: None,
        workspace: None,
        selection: ContextSelection::default(),
        library_user_id: Some(turn.library_user_id),
        chat: ChatTarget::Transient { messages: turn.messages },
    };
    send_with(app, payload, Some(forward))
}

/// The library user a published turn runs for, while it runs.
// The publication server, which asks for these turns, runs on Windows.
#[cfg(target_os = "windows")]
pub(crate) fn published_turn_user(app: &AppHandle, request_id: &str) -> Option<String> {
    let state = app.state::<AiChatState>();
    let turns = state.turns.lock().ok()?;
    turns
        .get(request_id)
        .filter(|turn| turn.forward.is_some())
        .map(|turn| turn.identity.context.actor.library_user_id.clone())
}

/// Answers the question a published turn waits on.
// The publication server, which asks for these turns, runs on Windows.
#[cfg(target_os = "windows")]
pub(crate) fn answer_published_turn(app: &AppHandle, request_id: &str, answer: InteractionAnswer) -> Result<(), BackendError> {
    answer_turn(&app.state::<AiChatState>(), request_id, answer)
}

/// Cancels a published turn: the question it waits on, or the running agent.
// The publication server, which asks for these turns, runs on Windows.
#[cfg(target_os = "windows")]
pub(crate) fn cancel_published_turn(app: &AppHandle, request_id: &str) -> Result<(), BackendError> {
    cancel_turn(app, request_id, false)
}

fn send_with(app: &AppHandle, payload: ChatSendPayload, forward: Option<TurnForward>) -> Result<ChatTurnOutcome, BackendError> {
    // The message as the person wrote it (it may be empty when they sent
    // only files) and the request the agent reads.
    let message = payload.message.trim().to_string();
    let request_text = chat_turn::turn_request(&message, payload.attachments.len())
        .ok_or_else(|| BackendError::invalid_input("El mensaje no puede estar vacío."))?;
    let title_seed = chat_turn::title_seed(
        &message,
        &payload.attachments.iter().map(|attachment| attachment.name.as_str()).collect::<Vec<_>>(),
    );
    let library_id = payload.library_id.clone();
    let runtime = app.state::<BackendRuntimeState>().inner().clone();
    if let Some(settings) = &payload.settings {
        runtime.configure_fallback(&settings.normalize())?;
    }

    // The chat and the messages the agent sees.
    let (chat, logical_path, history) = match payload.chat {
        ChatTarget::Saved { path } => {
            let logical_path = crate::chat_history::chat_logical_path(app, &library_id, &path)?;
            let document = crate::chat_history::load(app, &library_id, &logical_path, chat_turn::DEFAULT_CHAT_TITLE)?;
            let history = chat_turn::memory_window(&document).to_vec();
            (Some(document), Some(logical_path), history)
        }
        ChatTarget::Transient { messages } => (None, None, chat_turn::transient_window(&messages).to_vec()),
    };

    let workspace = workspace_of(payload.mode, payload.workspace)?;
    let (channel, scope, route_policy) = chat_turn::turn_route(payload.mode, &payload.scope, Some(&workspace.view));
    let persistence_policy = chat_turn::chat_persistence_policy(route_policy, chat.as_ref());
    let library_user_id = match payload.mode {
        TurnMode::Published => payload
            .library_user_id
            .filter(|user| !user.trim().is_empty())
            .ok_or_else(|| BackendError::invalid_input("La consulta publicada necesita un usuario de la biblioteca."))?,
        TurnMode::Chat | TurnMode::Meeting => OWNER_LIBRARY_USER_ID.to_string(),
    };
    let identity = RequestIdentity {
        idempotency_key: format!("{library_id}:{}", payload.request_id),
        context: BackendRequestContext {
            request_id: payload.request_id.clone(),
            library_id: library_id.clone(),
            actor: BackendActor { library_user_id, external_identity: None },
            channel,
            scope,
            persistence_policy,
        },
    };
    identity.context.validate()?;

    let state = app.state::<AiChatState>();
    {
        let mut turns = state.turns.lock().map_err(|_| internal("No se pudo iniciar la consulta."))?;
        if turns.contains_key(&payload.request_id) {
            return Err(BackendError::invalid_input("La consulta ya está en curso."));
        }
        turns.insert(
            payload.request_id.clone(),
            ActiveTurn {
                identity: identity.clone(),
                message: request_text.clone(),
                answer: None,
                cancelled: false,
                stopped_by_message: false,
                speaker: prompt_agent(payload.prompt_name.as_deref()),
                exchanges: Vec::new(),
                forward: forward.clone(),
            },
        );
    }
    // A published page follows the runtime's events of its turn.
    let _relay = forward.map(|forward| EventRelay::start(runtime.clone(), identity.context.clone(), forward));
    // Agents speak in the chats of the app, not in Meeting or published boards.
    let agents = match (&chat, payload.mode) {
        (Some(document), TurnMode::Chat) if payload.undo_operation_id.is_none() => {
            crate::chat_agents::chat_agents(app, &library_id, &document.agents)
        }
        _ => Vec::new(),
    };
    let user_message = StoredChatMessage {
        role: ChatRole::User,
        content: message.clone(),
        attachments: payload.attachments.clone(),
        agent: None,
    };
    // `/image` goes to the image model as written, without the agents.
    let image_command = match payload.mode {
        TurnMode::Chat => notia_backend_core::image_generation::parse_image_command(&message),
        TurnMode::Published | TurnMode::Meeting => None,
    };
    let result: Result<TurnReplies, BackendError> = match (payload.undo_operation_id.as_deref(), image_command) {
        (Some(operation_id), _) => undo(app, &state, &runtime, operation_id)
            .map(|path| (vec![notia_reply(chat_turn::undo_answer(path.as_deref()))], true, None)),
        (None, Some(command)) => Ok((vec![image_command_reply(app, &library_id, &payload.request_id, command, &payload.attachments)], false, None)),
        (None, None) => {
            let library_context = library_context(app, &library_id, payload.mode, &payload.scope, &payload.selection);
            let setup = TurnSetup {
                library_id: library_id.clone(),
                snapshot: chat_turn::workspace_snapshot(&workspace, &library_id),
                library_block: library_context.block,
                library_search: library_context.library_search,
                tool_access: chat_turn::chat_tool_access(chat.as_ref()),
                permanent_block: chat_turn::permanent_context_block(chat.as_ref()),
            };
            if agents.is_empty() {
                let prompt = setup.prompt(chat_turn::turn_prompt(payload.mode, &request_text, payload.context.as_deref()));
                let request = setup.request(&identity, &history, &prompt, &payload.attachments, payload.prompt_name.clone());
                let response = run_agent(app, &state, &runtime, &payload.request_id, &identity, request);
                let worked = remember_undoable(&state, &runtime, &identity);
                let response = response.map(|response| {
                    let changed = response.changed || response.tool_results.iter().any(|result| result.ok && result.changed);
                    let mut reply = notia_reply(response.response.markdown);
                    reply.agent = prompt_agent(payload.prompt_name.as_deref());
                    reply.attachments = generated_attachments(&payload.request_id);
                    (reply, changed)
                });
                single_agent_replies(take_exchanges(&state, &payload.request_id), response, worked, chat.is_some())
            } else {
                let dynamic = chat
                    .as_ref()
                    .and_then(|document| crate::chat_agents::chat_dynamic(app, &library_id, document.dynamic.as_deref()));
                let mut conversation = chat.as_ref().map(|document| document.messages.clone()).unwrap_or_default();
                // The agents read the request, also when the person sent only files.
                conversation.push(StoredChatMessage { content: request_text.clone(), ..user_message.clone() });
                let rounds =
                    run_agent_rounds(app, &state, &runtime, &identity, &setup, &agents, dynamic.as_deref(), &mut conversation);
                // What the agents said before an error or a cancel is kept.
                match rounds.error {
                    None => Ok((rounds.replies, rounds.changed, None)),
                    Some(error) => keep_failed_turn(rounds.replies, rounds.worked, error),
                }
            }
        }
    };
    let stopped_by_message = state
        .turns
        .lock()
        .ok()
        .and_then(|mut turns| turns.remove(&payload.request_id))
        .is_some_and(|turn| turn.stopped_by_message);
    let result = if stopped_by_message { closed_by_message(result) } else { result };
    let (replies, data_changed, interrupted) = result?;
    let answer = replies.last().map(|reply| reply.content.clone()).unwrap_or_default();

    // Save the turn in its chat.
    let document = match chat {
        Some(mut document) => {
            let previous_title = document.title.clone();
            let previous_messages = document.messages.clone();
            chat_turn::apply_turn_context(&mut document, &payload.selection);
            document.title = chat_turn::persisted_title(&previous_title, !previous_messages.is_empty(), &title_seed);
            document.messages.push(user_message);
            document.messages.extend(replies.iter().cloned());
            if let Some(logical_path) = &logical_path {
                if document.title != previous_title {
                    crate::chat_history::save_chat(app, &library_id, logical_path, &document)?;
                } else {
                    crate::chat_history::append_turn(app, &library_id, logical_path, &document, 1 + replies.len())?;
                }
            }
            // Memory and rules belong to the global engine: the agent reads
            // `rules.md` and `memory.md` and writes them only with its
            // `add_agent_rule` / `add_agent_memory` tools.
            if let Some(path) = logical_path.filter(|_| previous_messages.is_empty()) {
                schedule_chat_title(app, &library_id, path, title_seed.clone());
            }
            Some(document)
        }
        None => None,
    };
    // The messages are saved; the person still learns why the turn stopped.
    Ok(ChatTurnOutcome {
        answer,
        data_changed,
        document,
        undone_operation_id: payload.undo_operation_id,
        error: interrupted,
    })
}

/// A turn a message stopped is saved with a note instead of failing, so the
/// chat shows what happened and the next message keeps its context. What the
/// agents said before stays; the data may have changed before the cancel.
fn closed_by_message(result: Result<TurnReplies, BackendError>) -> Result<TurnReplies, BackendError> {
    let note = notia_reply(CANCELLED_REPLY.to_string());
    match result {
        Err(error) if error.code == BackendErrorCode::Cancelled => Ok((vec![note], true, None)),
        Ok((mut replies, _, Some(error))) if error.code == BackendErrorCode::Cancelled => {
            replies.push(note);
            Ok((replies, true, None))
        }
        other => other,
    }
}

/// Runs one chat turn; it returns when the agent answered and the turn was saved.
pub(crate) async fn ai_chat_send(app: AppHandle, payload: ChatSendPayload) -> Result<ChatTurnOutcome, BackendError> {
    crate::host::async_runtime::spawn_blocking(move || send(&app, payload))
        .await
        .map_err(|_| internal("La consulta terminó de forma inesperada."))?
}

/// The person's answer to the question the turn is waiting on.
pub(crate) fn ai_chat_answer(state: crate::host::State<'_, AiChatState>, payload: ChatAnswerPayload) -> Result<(), BackendError> {
    answer_turn(&state, &payload.request_id, payload.answer)
}

fn answer_turn(state: &AiChatState, request_id: &str, answer: InteractionAnswer) -> Result<(), BackendError> {
    let sender = state
        .turns
        .lock()
        .map_err(|_| internal("No se pudo responder la consulta."))?
        .get_mut(request_id)
        .and_then(|turn| turn.answer.take())
        .ok_or_else(|| BackendError::new(BackendErrorCode::Conflict, "La consulta ya no espera una respuesta.", false))?;
    sender
        .send(answer)
        .map_err(|_| BackendError::new(BackendErrorCode::Conflict, "La consulta ya no espera una respuesta.", false))
}

/// Cancels a turn: the question it waits on, or the running agent.
pub(crate) async fn ai_chat_cancel(app: AppHandle, payload: ChatCancelPayload) -> Result<(), BackendError> {
    crate::host::async_runtime::spawn_blocking(move || cancel_turn(&app, &payload.request_id, false))
        .await
        .map_err(|_| internal("No se pudo cancelar la consulta."))?
}

fn cancel_turn(app: &AppHandle, request_id: &str, by_message: bool) -> Result<(), BackendError> {
    let state = app.state::<AiChatState>();
    let identity = {
        let mut turns = state.turns.lock().map_err(|_| internal("No se pudo cancelar la consulta."))?;
        let Some(turn) = turns.get_mut(request_id) else {
            return Ok(());
        };
        turn.cancelled = true;
        turn.stopped_by_message |= by_message;
        // Dropping the pending answer lets the turn cancel its question.
        if turn.answer.take().is_some() {
            return Ok(());
        }
        turn.identity.clone()
    };
    let runtime = app.state::<BackendRuntimeState>().inner().clone();
    cancel_request(app, &runtime, &identity, None);
    Ok(())
}

/// A message the person sent while the turn `request_id` runs: a short call
/// to the model, beside the turn, decides whether it stops the turn, stops
/// it and goes next, or waits for the next turn. A turn that already ended
/// leaves the message for a new turn.
pub(crate) async fn ai_chat_interject(app: AppHandle, payload: ChatInterjectPayload) -> Result<ChatInterjectOutcome, BackendError> {
    crate::host::async_runtime::spawn_blocking(move || {
        let message = payload.message.trim().to_string();
        if message.is_empty() {
            return Err(BackendError::invalid_input("El mensaje no puede estar vacío."));
        }
        let running = app
            .state::<AiChatState>()
            .turns
            .lock()
            .map_err(|_| internal("No se pudo leer la consulta en curso."))?
            .get(&payload.request_id)
            .filter(|turn| !turn.cancelled)
            .map(|turn| (turn.message.clone(), turn.identity.context.clone()));
        let Some((running, context)) = running else {
            return Ok(ChatInterjectOutcome { decision: InterruptDecision::Queue });
        };
        let decision = crate::backend_runtime::classify_interrupt(&app, &context, &running, &message);
        if decision.cancels() {
            cancel_turn(&app, &payload.request_id, true)?;
        }
        Ok(ChatInterjectOutcome { decision })
    })
    .await
    .map_err(|_| internal("No se pudo decidir qué hacer con el mensaje."))?
}

#[cfg(test)]
mod tests {
    use super::{
        closed_by_message, exchange_messages, keep_failed_turn, notia_reply, plan_answer, plan_text, prompt_agent,
        single_agent_replies, strip_speaker_name, BackendError, BackendErrorCode, ChatRole, ExecutionPlan, CANCELLED_REPLY,
    };

    #[test]
    fn a_decided_plan_stays_in_the_chat_with_the_steps_kept_and_the_answer() {
        let step = |id: &str, label: &str| notia_backend_core::PlanStep {
            id: id.into(),
            label: label.into(),
            operation_id: None,
            status: notia_backend_core::PlanStepStatus::Pending,
        };
        let plan = ExecutionPlan {
            plan_id: "plan-1".into(),
            generation: 1,
            title: "Plan de ejecución".into(),
            status: notia_backend_core::PlanStatus::AwaitingApproval,
            steps: vec![step("s1", "Crear «Tecnología»"), step("s2", "Asignar Tecnología al celular"), step("s3", "Asignar Salud a la crema")],
        };
        assert_eq!(
            plan_text(&plan, None),
            "**Plan de ejecución**\n\n1. Crear «Tecnología»\n2. Asignar Tecnología al celular\n3. Asignar Salud a la crema"
        );
        let kept = ["s1".to_string(), "s2".to_string()];
        assert_eq!(plan_text(&plan, Some(&kept)), "**Plan de ejecución**\n\n1. Crear «Tecnología»\n2. Asignar Tecnología al celular");
        assert_eq!(plan_answer(true, None), "Aprobado.");
        assert_eq!(plan_answer(false, Some("  la crema va a Cuidado personal ")), "la crema va a Cuidado personal");
        assert_eq!(plan_answer(false, Some(" ")), "Cancelado.");
    }

    #[test]
    fn questions_answered_during_a_turn_are_kept_before_the_answer_or_with_the_error() {
        let exchanges = exchange_messages(" ¿Salud o Cuidado personal? ", "¿qué categoría recomendás?", Some("finanzas.md".into())).to_vec();
        assert_eq!((exchanges[0].role, exchanges[0].content.as_str()), (ChatRole::Assistant, "¿Salud o Cuidado personal?"));
        assert_eq!(exchanges[0].agent.as_deref(), Some("finanzas.md"));
        assert_eq!((exchanges[1].role, exchanges[1].agent.as_deref()), (ChatRole::User, None));

        let (replies, changed, error) =
            single_agent_replies(exchanges.clone(), Ok((notia_reply("Listo".into()), false)), true, true).expect("answer");
        assert_eq!(replies.iter().map(|reply| reply.content.as_str()).collect::<Vec<_>>(), ["¿Salud o Cuidado personal?", "¿qué categoría recomendás?", "Listo"]);
        assert!(!changed && error.is_none());

        let failure = || BackendError::new(BackendErrorCode::ProviderUnavailable, "sin IA", true);
        let (replies, _, error) = single_agent_replies(exchanges.clone(), Err(failure()), false, true).expect("kept with the error");
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[2].content, "No pude terminar: sin IA");
        assert_eq!(error.map(|error| error.code), Some(BackendErrorCode::ProviderUnavailable));
        // A chat that is not saved only reports the error.
        assert!(single_agent_replies(exchanges, Err(failure()), true, false).is_err());
    }

    #[test]
    fn a_turn_that_fails_after_working_stays_in_the_chat_and_one_that_did_nothing_goes_back() {
        let failure = || BackendError::invalid_input("El movimiento financiero no es válido.");
        // «dale» → the agent read and tried to save, then failed: kept with a note.
        let (replies, changed, error) = keep_failed_turn(Vec::new(), true, failure()).expect("kept");
        assert_eq!(replies.iter().map(|reply| reply.content.as_str()).collect::<Vec<_>>(), ["No pude terminar: El movimiento financiero no es válido."]);
        assert!(changed && error.is_some());
        // Nothing happened yet: the message goes back to the composer.
        assert!(keep_failed_turn(Vec::new(), false, failure()).is_err());
        // A cancel keeps what was said, without a note.
        let cancelled = BackendError::new(BackendErrorCode::Cancelled, "Consulta cancelada.", false);
        let (replies, _, error) = keep_failed_turn(vec![notia_reply("Ana: busco".into())], true, cancelled.clone()).expect("kept");
        assert_eq!(replies.len(), 1);
        assert_eq!(error.map(|error| error.code), Some(BackendErrorCode::Cancelled));
        assert!(keep_failed_turn(Vec::new(), true, cancelled).is_err());
    }

    #[test]
    fn a_turn_a_message_stopped_is_saved_with_a_note() {
        let cancelled = || BackendError::new(BackendErrorCode::Cancelled, "Consulta cancelada.", false);
        let (replies, changed, error) = closed_by_message(Err(cancelled())).expect("saved");
        assert_eq!(replies.iter().map(|reply| reply.content.as_str()).collect::<Vec<_>>(), [CANCELLED_REPLY]);
        assert!(changed && error.is_none());
        let (replies, _, error) =
            closed_by_message(Ok((vec![notia_reply("Ana: busco".into())], false, Some(cancelled())))).expect("saved");
        assert_eq!(replies.len(), 2);
        assert!(error.is_none());
        let failure = BackendError::new(BackendErrorCode::ProviderUnavailable, "sin IA", true);
        assert!(closed_by_message(Err(failure)).is_err());
    }

    #[test]
    fn a_custom_prompt_signs_its_reply_and_the_default_one_does_not() {
        assert_eq!(prompt_agent(Some("tasks.md")).as_deref(), Some("tasks.md"));
        assert_eq!(prompt_agent(Some("default.md")), None);
        assert_eq!(prompt_agent(Some("../x.md")), None);
        assert_eq!(prompt_agent(None), None);
    }

    #[test]
    fn an_agent_that_signs_its_message_loses_the_signature() {
        assert_eq!(strip_speaker_name("Ana: Hola", "Ana"), "Hola");
        assert_eq!(strip_speaker_name("Ana dice hola", "Ana"), "Ana dice hola");
        assert_eq!(strip_speaker_name("Hola", "Ana"), "Hola");
    }
}
