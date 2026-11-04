use serde::{Deserialize, Serialize};

use super::interaction::{ClarificationRequest, ExecutionPlan, OperationReview, OperationToken};
use super::{BackendError, BackendEvent, BackendRequestContext, BackendScope};

pub const MAX_BACKEND_PROTOCOL_VERSION: u16 = 2;
/// Largest serialized agent request. It carries the images of the turn (a
/// Telegram album of up to 10 photos, rendered PDF pages), so it matches the
/// 32 MiB the headless server accepts.
pub const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
/// Tools a routed request may carry: every tool the actor may use. Each
/// round offers only those of its areas, within `max_tools`.
pub const MAX_TOOL_POOL: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProtocolVersion(pub u16);

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self(MAX_BACKEND_PROTOCOL_VERSION)
    }
}

impl ProtocolVersion {
    pub fn validate(self) -> Result<(), BackendError> {
        if self.0 == 0 || self.0 > MAX_BACKEND_PROTOCOL_VERSION {
            return Err(BackendError::new(
                super::error::BackendErrorCode::Unsupported,
                "La versión del protocolo backend no está soportada.",
                false,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendSnapshot {
    pub snapshot_version: u16,
    pub view: String,
    pub scope: BackendScope,
    pub library_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_document: Option<DocumentSnapshot>,
    #[serde(default)]
    pub open_tabs: Vec<DocumentSnapshot>,
    pub capabilities: SnapshotCapabilities,
    pub captured_at: u64,
    /// Current editor selection of the active document, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectionSnapshot>,
}

/// Maximum open tabs carried by a snapshot.
pub const MAX_SNAPSHOT_TABS: usize = 100;
/// Maximum selected blocks carried by a snapshot.
pub const MAX_SELECTION_BLOCKS: usize = 50;
/// Maximum characters of all selected block texts together.
pub const MAX_SELECTION_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSnapshot {
    pub document_path: String,
    pub from: u64,
    pub to: u64,
    #[serde(default)]
    pub blocks: Vec<SelectionBlockSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionBlockSnapshot {
    pub index: u32,
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
}

impl BackendSnapshot {
    /// The snapshot is client-supplied visual context: it must belong to the
    /// request library, stay bounded and only describe the active document's
    /// selection. It never grants access by itself.
    pub fn validate(&self, context: &super::BackendRequestContext) -> Result<(), BackendError> {
        if self.library_id != context.library_id {
            return Err(BackendError::new(
                super::BackendErrorCode::Forbidden,
                "El snapshot no pertenece a la biblioteca de la solicitud.",
                false,
            ));
        }
        let bounded = |value: &str, max: usize| {
            value.chars().count() <= max && !value.chars().any(char::is_control)
        };
        if !bounded(&self.view, 64) || self.open_tabs.len() > MAX_SNAPSHOT_TABS {
            return Err(BackendError::invalid_input("El snapshot supera los límites permitidos."));
        }
        let documents = self.active_document.iter().chain(self.open_tabs.iter());
        for document in documents {
            if !bounded(&document.path, 2_048)
                || !bounded(&document.name, 512)
                || !bounded(&document.kind, 64)
            {
                return Err(BackendError::invalid_input("El snapshot contiene un documento inválido."));
            }
        }
        if let Some(selection) = &self.selection {
            let active_path = self.active_document.as_ref().map(|document| document.path.as_str());
            if active_path != Some(selection.document_path.as_str()) {
                return Err(BackendError::invalid_input(
                    "La selección no corresponde al documento activo.",
                ));
            }
            let total_chars = selection
                .blocks
                .iter()
                .map(|block| block.text.chars().count())
                .sum::<usize>();
            if selection.from > selection.to
                || selection.blocks.len() > MAX_SELECTION_BLOCKS
                || total_chars > MAX_SELECTION_CHARS
                || selection.blocks.iter().any(|block| !bounded(&block.block_type, 64))
            {
                return Err(BackendError::invalid_input("La selección supera los límites permitidos."));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCapabilities {
    pub can_read_active_document: bool,
    pub can_read_library: bool,
    pub can_write_active_document: bool,
    pub can_write_library: bool,
    pub can_search_web: bool,
    pub can_ask_clarification: bool,
    pub can_request_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSnapshot {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub revision: u64,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
    pub kind: AttachmentKind,
    pub media_type: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub scopes: Vec<BackendScope>,
    pub read_only: bool,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub round: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BackendError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MutationPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationPreview {
    pub operation_id: String,
    pub summary: String,
    pub documents: Vec<PreviewDocument>,
    pub hunks: Vec<PreviewHunk>,
    pub allowed_actions: Vec<MutationPreviewAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDocument {
    pub path: String,
    pub expected_revision: u64,
    pub current_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewHunk {
    pub id: String,
    pub document_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MutationPreviewAction {
    ApplyAll,
    ApplySelected,
    Reject,
    Edit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub context: BackendRequestContext,
    pub messages: Vec<BackendMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<BackendSnapshot>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    pub idempotency_key: String,
    /// Custom prompt file under `.agent/promps` selected by the user. It only
    /// adds preferences; it can never replace the embedded base prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_name: Option<String>,
    /// Tools the person allows for this request, on top of what its actor
    /// and scope allow.
    #[serde(default)]
    pub tool_access: ToolAccess,
    /// The agent may search and read the whole library. Off, the tools that
    /// read or search it are left out and the turn keeps only the files the
    /// person chose.
    #[serde(default = "library_search_default")]
    pub library_search: bool,
    /// Notia started the run by itself (the hourly review or a new mail),
    /// not a message of the person: it only reads, keeps its thoughts and
    /// answers with the message to send, if any (see `agent_autonomy`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autonomous: bool,
    /// The run of an AI action (see `ai_actions`): a turn of the Owner's
    /// Telegram chat whose request is the action's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_action: Option<crate::ai_actions::prompts::ScheduledActionPrompt>,
    /// The run of a webhook call (see `webhooks`): a turn of the Owner's
    /// Telegram chat whose request is the prompt another program sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_call: Option<crate::webhooks::WebhookCallPrompt>,
    /// Tool areas routed for this run (see `tool_routing`). With them,
    /// `tools` holds every tool the actor may use and each round offers the
    /// tools of these areas; the agent changes them with `change_tool_areas`.
    /// Without them, `tools` is offered as is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_areas: Option<Vec<String>>,
    /// Whether the run thinks, as the routing call decided for the request
    /// (see `tool_routing`): `Some(false)` turns the configured thinking off
    /// for a conversational turn; it never turns on what the settings left
    /// off. `None` keeps the settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    /// The routing call of a Telegram turn chose the conversational default
    /// prompt (see `prompt::base_agent_prompt`): the person shares something
    /// of their life or wants to chat. A resumed run keeps it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conversational: bool,
    /// A phase of an AI task (see `ai_tasks`): the run gets the phase's
    /// prompt section and tools, and never creates other tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_task: Option<crate::ai_tasks::AiTaskTurn>,
}

fn library_search_default() -> bool {
    true
}

/// Tools a chat allows. Memory tools depend on the memory policy, not on
/// this: a chat with memory keeps them even without other tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolAccess {
    #[default]
    All,
    /// Only tools that do not change library data.
    ReadOnly,
    /// No tools.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Files attached to the message; the backend quotes their text and
    /// appends their pages to the images sent to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<super::chat_attachments::MessageAttachment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "kebab-case")]
pub enum BackendRequest {
    Run(AgentRequest),
    Resume(ResumeRequest),
    Cancel(CancelRequest),
    GetOperation(GetOperationRequest),
    Review(ReviewRequest),
    Undo(UndoOperationRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeRequest {
    pub context: BackendRequestContext,
    pub idempotency_key: String,
    pub request_id: String,
    pub operation: OperationToken,
    pub last_event_sequence: u64,
    pub decision: ResumeDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelRequest {
    pub context: BackendRequestContext,
    pub idempotency_key: String,
    pub request_id: String,
    /// Absent when cancelling a running request that has no interaction yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOperationRequest {
    pub context: BackendRequestContext,
    pub idempotency_key: String,
    pub request_id: String,
    /// Absent when a reconnecting client re-attaches by request identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationToken>,
    #[serde(default)]
    pub last_event_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub context: BackendRequestContext,
    pub idempotency_key: String,
    pub request_id: String,
    pub operation: OperationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoOperationRequest {
    pub context: BackendRequestContext,
    pub idempotency_key: String,
    pub request_id: String,
    pub operation: OperationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "kebab-case")]
pub enum ResumeDecision {
    Clarification(super::interaction::ClarificationAnswer),
    Confirmation(ConfirmationDecision),
    Plan(PlanDecision),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDecision {
    pub plan_id: String,
    pub generation: u64,
    pub accepted: bool,
    #[serde(default)]
    pub step_ids: Vec<String>,
    /// Changes the person asked for instead of approving the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// «Confirmar todos»: the plan and every change the rest of this
    /// request asks for are approved; later requests ask again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approve_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "kebab-case")]
pub enum BackendResponse {
    Accepted {
        request_id: String,
    },
    Operation {
        status: OperationStatus,
    },
    Resumed {
        status: OperationStatus,
        decision: ResumeDecision,
    },
    Review {
        review: OperationReview,
    },
    Undo {
        result: super::interaction::UndoResult,
    },
    Result {
        response: AgentResponse,
    },
    Cancelled {
        request_id: String,
    },
    Error {
        request_id: String,
        error: BackendError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStatus {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationToken>,
    pub state: OperationState,
    pub last_event_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<AgentResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<PendingInteraction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<OperationReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "kebab-case")]
pub enum PendingInteraction {
    Clarification(ClarificationRequest),
    Confirmation(super::interaction::ConfirmationRequest),
    Plan(ExecutionPlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    Pending,
    Running,
    WaitingClarification,
    WaitingConfirmation,
    WaitingPlan,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResponse {
    pub request_id: String,
    pub changed: bool,
    pub rounds: u32,
    pub response: ChannelResponse,
    #[serde(default)]
    pub tool_results: Vec<ToolResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelResponse {
    pub markdown: String,
    pub telegram_html: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationDecision {
    pub operation_id: String,
    pub accepted: bool,
    #[serde(default)]
    pub hunk_ids: Vec<String>,
    /// «Confirmar todos»: this change and every other one the rest of this
    /// request asks for are approved; later requests ask again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approve_all: bool,
    /// «Proponer otra cosa»: the change is not made and the agent goes on
    /// with what the person proposes instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendEventEnvelope {
    pub protocol_version: ProtocolVersion,
    pub request_id: String,
    pub sequence: u64,
    pub event: BackendEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRequestEnvelope {
    pub protocol_version: ProtocolVersion,
    pub request: BackendRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendResponseEnvelope {
    pub protocol_version: ProtocolVersion,
    pub response: BackendResponse,
}

impl BackendRequest {
    pub fn validate(&self, limits: &BackendLimits) -> Result<(), BackendError> {
        match self {
            Self::Run(request) => limits.validate_request(request),
            Self::Resume(request) => {
                validate_interaction_request(&request.context, &request.idempotency_key)?;
                validate_request_id(&request.request_id)?;
                validate_request_id_match(&request.context, &request.request_id)?;
                request.operation.validate()
            }
            Self::Cancel(request) => validate_optional_operation_request(
                &request.context,
                &request.idempotency_key,
                &request.request_id,
                request.operation.as_ref(),
            ),
            Self::GetOperation(request) => validate_optional_operation_request(
                &request.context,
                &request.idempotency_key,
                &request.request_id,
                request.operation.as_ref(),
            ),
            Self::Review(request) => validate_operation_request(
                &request.context,
                &request.idempotency_key,
                &request.request_id,
                &request.operation,
            ),
            Self::Undo(request) => validate_operation_request(
                &request.context,
                &request.idempotency_key,
                &request.request_id,
                &request.operation,
            ),
        }
    }
}

fn validate_interaction_request(
    context: &BackendRequestContext,
    idempotency_key: &str,
) -> Result<(), BackendError> {
    context.validate()?;
    if idempotency_key.trim().is_empty()
        || idempotency_key.chars().count() > 256
        || idempotency_key.chars().any(char::is_control)
    {
        return Err(BackendError::invalid_input("idempotencyKey no es válido."));
    }
    Ok(())
}

fn validate_operation_request(
    context: &BackendRequestContext,
    idempotency_key: &str,
    request_id: &str,
    operation: &OperationToken,
) -> Result<(), BackendError> {
    validate_interaction_request(context, idempotency_key)?;
    validate_request_id(request_id)?;
    validate_request_id_match(context, request_id)?;
    operation.validate()
}

fn validate_optional_operation_request(
    context: &BackendRequestContext,
    idempotency_key: &str,
    request_id: &str,
    operation: Option<&OperationToken>,
) -> Result<(), BackendError> {
    validate_interaction_request(context, idempotency_key)?;
    validate_request_id(request_id)?;
    validate_request_id_match(context, request_id)?;
    operation.map_or(Ok(()), OperationToken::validate)
}

fn validate_request_id_match(
    context: &BackendRequestContext,
    request_id: &str,
) -> Result<(), BackendError> {
    if context.request_id != request_id {
        return Err(BackendError::invalid_input(
            "requestId no coincide con el contexto de la operación.",
        ));
    }
    Ok(())
}

fn validate_request_id(request_id: &str) -> Result<(), BackendError> {
    let value = request_id.trim();
    if value.is_empty() || value.chars().count() > 128 || value.chars().any(char::is_control) {
        return Err(BackendError::invalid_input("requestId no es válido."));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendLimits {
    pub max_messages: usize,
    pub max_message_chars: usize,
    pub max_tools: usize,
    pub max_attachments: usize,
    pub max_images: usize,
    pub max_documents: usize,
    pub max_attachment_bytes: u64,
    pub max_result_bytes: usize,
    pub max_request_bytes: usize,
}

impl Default for BackendLimits {
    fn default() -> Self {
        Self {
            max_messages: 128,
            max_message_chars: 100_000,
            // Finanzas offers 77 tools; 96 keeps room and stays under the 128
            // tools the strictest model APIs accept.
            max_tools: 96,
            max_attachments: 16,
            max_images: 8,
            max_documents: 8,
            max_attachment_bytes: 25 * 1024 * 1024,
            max_result_bytes: 2 * 1024 * 1024,
            max_request_bytes: MAX_REQUEST_BYTES,
        }
    }
}

impl BackendLimits {
    pub fn validate_request(&self, request: &AgentRequest) -> Result<(), BackendError> {
        request.context.validate()?;
        if let Some(snapshot) = &request.snapshot {
            snapshot.validate(&request.context)?;
        }
        if request.idempotency_key.trim().is_empty()
            || request.idempotency_key.chars().count() > 256
            || request.idempotency_key.chars().any(char::is_control)
        {
            return Err(BackendError::invalid_input("idempotencyKey no es válido."));
        }
        if request.messages.len() > self.max_messages
            || request
                .messages
                .iter()
                .any(|message| message.content.chars().count() > self.max_message_chars)
        {
            return Err(BackendError::invalid_input(
                "El request supera el límite de mensajes.",
            ));
        }
        for message in &request.messages {
            super::chat_attachments::validate_attachments(&message.attachments)?;
        }
        let tool_limit = if request.tool_areas.is_some() { MAX_TOOL_POOL } else { self.max_tools };
        if request.tools.len() > tool_limit {
            return Err(BackendError::invalid_input(
                "El request supera el límite de tools.",
            ));
        }
        if request.tools.iter().any(|tool| {
            tool.name.trim().is_empty()
                || tool.name.chars().count() > 128
                || tool.name.chars().any(char::is_control)
        }) {
            return Err(BackendError::invalid_input(
                "El catálogo contiene una tool con nombre inválido.",
            ));
        }
        if request.attachments.len() > self.max_attachments
            || request
                .attachments
                .iter()
                .any(|attachment| attachment.byte_length > self.max_attachment_bytes)
        {
            return Err(BackendError::invalid_input(
                "El request supera el límite de adjuntos.",
            ));
        }
        let image_count = request
            .attachments
            .iter()
            .filter(|attachment| attachment.kind == AttachmentKind::Image)
            .count();
        let document_count = request
            .attachments
            .iter()
            .filter(|attachment| attachment.kind == AttachmentKind::Document)
            .count();
        if image_count > self.max_images || document_count > self.max_documents {
            return Err(BackendError::invalid_input(
                "El request supera el límite de imágenes o documentos.",
            ));
        }
        let request_bytes = serde_json::to_vec(request).map_err(|_| {
            BackendError::invalid_input("El request no puede serializarse de forma segura.")
        })?;
        if request_bytes.len() > self.max_request_bytes {
            return Err(BackendError::invalid_input(
                "El payload del request es demasiado grande.",
            ));
        }
        Ok(())
    }

    /// `result` as the model receives it: one larger than `max_result_bytes`
    /// keeps the start of its data and a note to ask for less, instead of
    /// ending the turn.
    pub fn fit_result(&self, result: ToolResult) -> ToolResult {
        let Ok(bytes) = serde_json::to_vec(&result) else {
            return ToolResult {
                ok: false,
                data: None,
                error: Some(BackendError::invalid_input("El resultado no puede serializarse de forma segura.")),
                preview: None,
                ..result
            };
        };
        if bytes.len() <= self.max_result_bytes {
            return result;
        }
        let data = result.data.as_ref().map(serde_json::Value::to_string).unwrap_or_default();
        // Room for the note and the JSON escaping of the kept text.
        let mut keep = (self.max_result_bytes / 3).min(data.len());
        while !data.is_char_boundary(keep) {
            keep -= 1;
        }
        ToolResult {
            data: Some(serde_json::json!({
                "truncated": true,
                "totalBytes": bytes.len(),
                "partialResult": &data[..keep],
                "note": "El resultado era demasiado grande y llega cortado: arriba va su comienzo. Pedí menos para ver el resto (de a un documento, con offset y limit, o una búsqueda más precisa); no afirmes nada sobre lo que no llegó.",
            })),
            preview: None,
            ..result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendActor, BackendChannel, PersistencePolicy};

    fn request() -> AgentRequest {
        AgentRequest {
            context: BackendRequestContext {
                request_id: "request-1".to_string(),
                library_id: "library-1".to_string(),
                actor: BackendActor {
                    library_user_id: "user-1".to_string(),
                    external_identity: None,
                },
                channel: BackendChannel::App,
                scope: BackendScope::Library,
                persistence_policy: PersistencePolicy::Persistent,
            },
            messages: vec![BackendMessage {
                role: MessageRole::User,
                content: "hola".to_string(),
                images: Vec::new(),
                attachments: Vec::new(),
            }],
            snapshot: None,
            tools: Vec::new(),
            attachments: Vec::new(),
            idempotency_key: "request-1".to_string(),
            prompt_name: None,
            tool_access: Default::default(),
            library_search: true,
            autonomous: false,
            scheduled_action: None,
            webhook_call: None,
            tool_areas: None,
            thinking: None,
            conversational: false,
            ai_task: None,
        }
    }

    #[test]
    fn serializes_versioned_requests_and_ignores_unknown_fields() {
        let value =
            serde_json::to_value(BackendRequest::Run(request())).expect("request serializes");
        assert_eq!(value["type"], "run");
        assert_eq!(value["payload"]["context"]["requestId"], "request-1");

        let mut compatible = value;
        compatible["unknownFutureField"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BackendRequest>(compatible).is_ok());
    }

    #[test]
    fn validates_all_transport_request_variants() {
        let limits = BackendLimits::default();
        assert!(BackendRequest::Run(request()).validate(&limits).is_ok());
        assert!(BackendRequest::Resume(ResumeRequest {
            context: request().context,
            idempotency_key: "request-1".to_string(),
            request_id: "request-1".to_string(),
            operation: crate::OperationToken {
                operation_id: "operation-1".into(),
                generation: 1,
            },
            last_event_sequence: 0,
            decision: ResumeDecision::Plan(PlanDecision {
                plan_id: "plan-1".into(),
                generation: 1,
                accepted: true,
                step_ids: Vec::new(),
                suggestion: None,
                approve_all: false,
            }),
        })
        .validate(&limits)
        .is_ok());
        assert!(BackendRequest::Cancel(CancelRequest {
            context: request().context,
            idempotency_key: "request-1".into(),
            request_id: "  ".to_string(),
            operation: Some(crate::OperationToken {
                operation_id: "operation-1".into(),
                generation: 1,
            }),
        })
        .validate(&limits)
        .is_err());
    }

    #[test]
    fn rejects_unsupported_protocol_versions() {
        assert!(ProtocolVersion(0).validate().is_err());
        assert!(ProtocolVersion(MAX_BACKEND_PROTOCOL_VERSION + 1)
            .validate()
            .is_err());
    }

    #[test]
    fn rejects_oversized_payloads_before_transport() {
        let mut limits = BackendLimits::default();
        limits.max_message_chars = 3;
        assert!(limits.validate_request(&request()).is_err());
    }

    #[test]
    fn a_result_too_large_arrives_cut_instead_of_failing() {
        let limits = BackendLimits { max_result_bytes: 4_000, ..BackendLimits::default() };
        let result = |text: String| ToolResult {
            call_id: "call-1".to_string(),
            ok: true,
            changed: false,
            data: Some(serde_json::json!({ "value": text })),
            error: None,
            preview: None,
        };
        let small = result("ñandú".to_string());
        assert_eq!(limits.fit_result(small.clone()), small);
        let cut = limits.fit_result(result("ñandú ".repeat(5_000)));
        assert!(cut.ok && cut.call_id == "call-1");
        assert!(serde_json::to_vec(&cut).expect("serializes").len() <= limits.max_result_bytes);
        let data = cut.data.expect("data");
        assert_eq!(data["truncated"], true);
        assert!(data["partialResult"].as_str().is_some_and(|text| text.starts_with("{\"value\":\"ñandú")));
    }

    #[test]
    fn serializes_resume_and_channel_specific_responses() {
        let response = BackendResponse::Result {
            response: AgentResponse {
                request_id: "request-1".to_string(),
                changed: false,
                rounds: 1,
                response: ChannelResponse {
                    markdown: "respuesta".to_string(),
                    telegram_html: "respuesta".to_string(),
                    data: serde_json::json!({"ok": true}),
                },
                tool_results: Vec::new(),
            },
        };
        let value = serde_json::to_value(response).expect("response serializes");
        assert_eq!(value["type"], "result");
        assert_eq!(
            value["payload"]["response"]["response"]["telegramHtml"],
            "respuesta"
        );
        assert_eq!(
            serde_json::to_value(BackendRequest::Resume(ResumeRequest {
                context: request().context,
                idempotency_key: "request-1".into(),
                request_id: "request-1".to_string(),
                operation: crate::OperationToken {
                    operation_id: "operation-1".into(),
                    generation: 1,
                },
                last_event_sequence: 4,
                decision: ResumeDecision::Confirmation(ConfirmationDecision {
                    operation_id: "operation-1".into(),
                    accepted: true,
                    hunk_ids: Vec::new(),
                    approve_all: false,
                    suggestion: None,
                }),
            }))
            .expect("resume serializes")["payload"]["lastEventSequence"],
            4
        );
    }

    #[test]
    fn accepts_the_shared_v1_fixture() {
        let fixture = include_str!("../fixtures/protocol-v1-run.json");
        let envelope: BackendRequestEnvelope =
            serde_json::from_str(fixture).expect("shared protocol fixture deserializes");
        assert_eq!(envelope.protocol_version, ProtocolVersion(1));
        assert!(envelope.request.validate(&BackendLimits::default()).is_ok());
    }

    #[test]
    fn snapshot_must_match_the_library_and_the_active_document() {
        let mut request = request();
        let mut snapshot = BackendSnapshot {
            snapshot_version: 1,
            view: "editor".into(),
            scope: BackendScope::Document,
            library_id: request.context.library_id.clone(),
            active_document: Some(DocumentSnapshot {
                path: "a.md".into(),
                name: "a.md".into(),
                kind: "markdown".into(),
                revision: 1,
                dirty: false,
            }),
            open_tabs: Vec::new(),
            capabilities: SnapshotCapabilities::default(),
            captured_at: 0,
            selection: Some(SelectionSnapshot {
                document_path: "a.md".into(),
                from: 0,
                to: 4,
                blocks: Vec::new(),
            }),
        };
        request.snapshot = Some(snapshot.clone());
        assert!(BackendLimits::default().validate_request(&request).is_ok());

        snapshot.selection.as_mut().expect("selection").document_path = "b.md".into();
        request.snapshot = Some(snapshot.clone());
        assert!(BackendLimits::default().validate_request(&request).is_err());

        snapshot.selection = None;
        snapshot.library_id = "other-library".into();
        request.snapshot = Some(snapshot);
        assert!(BackendLimits::default().validate_request(&request).is_err());
    }
}
