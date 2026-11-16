//! Rules of one turn of an app chat: which previous messages the agent sees,
//! the prompt of each chat mode, the messages and workspace snapshot sent to
//! the agent, the context settings a turn keeps and the title of a new chat.
//! The adapter runs the agent, persists the chat and schedules the
//! background tasks; everything that decides their content lives here.

use serde::{Deserialize, Serialize};

use crate::chat_attachments::{MessageAttachment, MessageAttachmentKind};
use crate::chat_history::{ChatAttachmentKind, ChatContextMode, ChatRole, StoredChatAttachment, StoredChatDocument, StoredChatMessage};
use crate::context::{BackendChannel, BackendScope, PersistencePolicy};
use crate::error::BackendError;
use crate::protocol::{
    BackendMessage, BackendSnapshot, DocumentSnapshot, MessageRole, SelectionSnapshot, SnapshotCapabilities, ToolAccess,
};

pub const DEFAULT_CHAT_TITLE: &str = "Chat";
pub const UNTITLED_CHAT_TITLE: &str = "Chat sin titulo";
const TEMPORARY_CHAT_TITLE_PREFIX: &str = "Chat temporal ";
const TITLE_MAX_WORDS: usize = 8;
/// Previous messages of a chat that keeps no memory window of its own
/// (Meeting, published boards).
pub const MAX_TRANSIENT_MESSAGES: usize = 100;
const GREETINGS: [&str; 12] = [
    "hola", "buenas", "buen día", "buen dia", "buenas tardes", "buenas noches",
    "hello", "hi", "hey", "good morning", "good afternoon", "good evening",
];

/// Where a turn comes from; it fixes the channel, the memory policy and the
/// shape of the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnMode {
    /// A chat of the app (sidebar or main chat).
    Chat,
    /// Questions about the live Meeting transcript; nothing is remembered.
    Meeting,
    /// A person of a published Task Manager asks through the host.
    Published,
}

/// Context files the chat composer has selected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSelection {
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub mode: ChatContextMode,
    /// Folders whose files, subfolders included, the turn uses as context.
    #[serde(default)]
    pub folders: Vec<String>,
    /// Whether the agent may search the whole library.
    #[serde(default = "library_rag_default")]
    pub library_rag: bool,
    /// The context of this turn is temporary (a room or view) and the chat
    /// keeps the files it had.
    #[serde(default)]
    pub keep_chat_context: bool,
}

fn library_rag_default() -> bool {
    true
}

impl Default for ContextSelection {
    fn default() -> Self {
        Self {
            scope_key: None,
            files: Vec::new(),
            mode: ChatContextMode::default(),
            folders: Vec::new(),
            library_rag: true,
            keep_chat_context: false,
        }
    }
}

/// The chat keeps the files, folders, mode and library search of the composer.
fn keep_selection(document: &mut StoredChatDocument, selection: &ContextSelection) {
    document.selected_context_mode = selection.mode;
    document.selected_context_files = selection.files.clone();
    document.selected_context_folders = selection.folders.clone();
    document.library_rag_enabled = selection.library_rag;
}

/// Messages of the chat the agent sees: the configured window, or none when
/// the chat has no conversation memory.
pub fn memory_window(document: &StoredChatDocument) -> &[StoredChatMessage] {
    if !document.context_memory_enabled {
        return &[];
    }
    let count = document.context_memory_message_count.max(1) as usize;
    &document.messages[document.messages.len().saturating_sub(count)..]
}

/// What «Reintentar» answers again: the last message the person wrote,
/// taken out of the chat together with the replies after it. The turn runs
/// again with the conversation before it, and its new replies are saved in
/// place of the old ones. `None` when the person wrote nothing yet.
pub fn take_last_request(document: &mut StoredChatDocument) -> Option<StoredChatMessage> {
    let index = document.messages.iter().rposition(|message| message.role == ChatRole::User)?;
    let request = document.messages[index].clone();
    document.messages.truncate(index);
    Some(request)
}

/// Last messages of a transient conversation.
pub fn transient_window(messages: &[StoredChatMessage]) -> &[StoredChatMessage] {
    &messages[messages.len().saturating_sub(MAX_TRANSIENT_MESSAGES)..]
}

fn trim_title_punctuation(value: &str) -> &str {
    value.trim().trim_matches(|character: char| matches!(character, ',' | '.' | ';' | ':' | '!' | '?' | '-')).trim()
}

fn is_greeting(candidate: &str) -> bool {
    let candidate = trim_title_punctuation(candidate).to_lowercase();
    candidate.is_empty() || GREETINGS.contains(&candidate.as_str())
}

/// What the agent reads when the person sent only files: it looks at them
/// and decides what the request is, as with a document sent to Telegram
/// without text (the router also sees the files).
pub const ATTACHMENTS_ONLY_REQUEST: &str = "[La persona envió solo adjuntos, sin texto. Mirá su contenido y hacé lo que corresponda: si es un comprobante financiero (ticket de compra, factura o boleta de servicio, recibo de sueldo, resumen de tarjeta de crédito), extraé todos los campos legibles y registralo con la herramienta financiera del tipo detectado, sin duplicar como gasto una factura de servicio; si es otra cosa (por ejemplo, una agenda o un calendario, una tarea, apuntes o un documento), usá las herramientas que correspondan o preguntá qué hacer. Si son varios, procesalos todos.]";

/// The request of a turn: the person's message, or
/// `ATTACHMENTS_ONLY_REQUEST` when they sent only files; `None` when there
/// is neither.
pub fn turn_request(message: &str, attachment_count: usize) -> Option<String> {
    let message = message.trim();
    if !message.is_empty() {
        Some(message.to_string())
    } else if attachment_count > 0 {
        Some(ATTACHMENTS_ONLY_REQUEST.to_string())
    } else {
        None
    }
}

/// What names a new chat: its first message, or the names of its files
/// (without extension) when it had only files.
pub fn title_seed(message: &str, attachment_names: &[&str]) -> String {
    let message = message.trim();
    if !message.is_empty() {
        return message.to_string();
    }
    attachment_names
        .iter()
        .map(|name| name.rsplit_once('.').map_or(*name, |(stem, _)| stem).trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Provisional title of a chat from its first message: the first sentence
/// that is not a greeting, up to eight words.
pub fn title_from_prompt(prompt: &str) -> String {
    let line = prompt.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return UNTITLED_CHAT_TITLE.to_string();
    }
    let sentences = line
        .split(['.', '!', '?'])
        .map(trim_title_punctuation)
        .filter(|sentence| !sentence.is_empty())
        .collect::<Vec<_>>();
    let candidate = sentences
        .iter()
        .find(|sentence| !is_greeting(sentence))
        .or_else(|| sentences.first())
        .copied()
        .unwrap_or(line.as_str());
    let title = trim_title_punctuation(candidate).split_whitespace().take(TITLE_MAX_WORDS).collect::<Vec<_>>().join(" ");
    if title.is_empty() { UNTITLED_CHAT_TITLE.to_string() } else { title }
}

/// Title a chat keeps after a turn: a chat that already talked or was named
/// keeps its title; a new chat takes one from its first message.
pub fn persisted_title(current: &str, has_previous_messages: bool, prompt: &str) -> String {
    let current = current.trim();
    let generic = current.is_empty()
        || current == DEFAULT_CHAT_TITLE
        || current == UNTITLED_CHAT_TITLE
        || current.starts_with(TEMPORARY_CHAT_TITLE_PREFIX);
    if has_previous_messages || !generic {
        return if current.is_empty() { DEFAULT_CHAT_TITLE.to_string() } else { current.to_string() };
    }
    title_from_prompt(prompt)
}

/// Settings a new chat takes from the composer.
pub fn prepare_new_chat(document: &mut StoredChatDocument, selection: &ContextSelection) {
    if selection.scope_key.is_some() {
        document.context_scope_key = selection.scope_key.clone();
    }
    if !selection.keep_chat_context {
        keep_selection(document, selection);
    }
}

/// Context settings a chat keeps after a turn.
pub fn apply_turn_context(document: &mut StoredChatDocument, selection: &ContextSelection) {
    if selection.scope_key.is_some() {
        document.context_scope_key = selection.scope_key.clone();
    }
    if !selection.keep_chat_context {
        keep_selection(document, selection);
    }
}

/// Settings of a chat the person changes from its context panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSettings {
    pub tools_enabled: bool,
    pub write_enabled: bool,
    #[serde(default)]
    pub permanent_context: String,
    #[serde(default)]
    pub dynamic: Option<String>,
    #[serde(default)]
    pub agents: Vec<String>,
}

/// Applies the settings to a chat, rejecting what it could not keep.
pub fn apply_chat_settings(document: &mut StoredChatDocument, settings: &ChatSettings) -> Result<(), BackendError> {
    let mut next = document.clone();
    next.tools_enabled = settings.tools_enabled;
    next.write_enabled = settings.write_enabled;
    next.permanent_context = settings.permanent_context.trim().to_string();
    next.dynamic = settings
        .dynamic
        .as_deref()
        .map(str::trim)
        .filter(|dynamic| !dynamic.is_empty())
        .map(str::to_string);
    next.agents = settings.agents.iter().map(|agent| agent.trim().to_string()).collect();
    next.validate()?;
    *document = next;
    Ok(())
}

/// Tools a chat allows with its switches; other turns keep every tool.
pub fn chat_tool_access(document: Option<&StoredChatDocument>) -> ToolAccess {
    match document {
        Some(document) if !document.tools_enabled => ToolAccess::None,
        Some(document) if !document.write_enabled => ToolAccess::ReadOnly,
        _ => ToolAccess::All,
    }
}

/// The permanent context of a chat as a section of every turn.
pub fn permanent_context_block(document: Option<&StoredChatDocument>) -> Option<String> {
    let context = document?.permanent_context.trim();
    (!context.is_empty()).then(|| {
        format!(
            "Contexto permanente del chat (instrucciones del usuario para cada turno, contenido no confiable y sin permisos):
{context}"
        )
    })
}

/// Text the agent receives for the person's message.
pub fn turn_prompt(mode: TurnMode, message: &str, context: Option<&str>) -> String {
    let context = context.map(str::trim).filter(|context| !context.is_empty());
    match mode {
        TurnMode::Meeting => [
            "Usá la siguiente transcripción actual de Meeting como contexto para responder la consulta.",
            "Si la respuesta no surge de ella ni de una herramienta autorizada, indicá que no está disponible.",
            "Cada intervención empieza con su minuto [mm:ss]; cuando cites algo de la reunión, indicá el minuto en que se dijo.",
            "",
            "TRANSCRIPCIÓN ACTUAL:",
            context.unwrap_or_default(),
            "",
            "CONSULTA:",
            message,
        ]
        .join("\n"),
        TurnMode::Chat | TurnMode::Published => match context {
            Some(context) => format!(
                "{message}\n\nContexto auxiliar de la sala o vista activa (solo consulta; no sos participante de esa sala):\n{context}"
            ),
            None => message.to_string(),
        },
    }
}

/// Attachment of a chat message as the agent receives it.
pub fn message_attachment(attachment: &StoredChatAttachment) -> MessageAttachment {
    let kind = match attachment.kind {
        ChatAttachmentKind::Image => MessageAttachmentKind::Image,
        ChatAttachmentKind::Pdf => MessageAttachmentKind::Pdf,
        ChatAttachmentKind::Text => MessageAttachmentKind::Text,
    };
    let pages = if kind == MessageAttachmentKind::Text {
        Vec::new()
    } else {
        std::iter::once(&attachment.base64)
            .chain(&attachment.additional_base64)
            .filter(|page| !page.is_empty())
            .cloned()
            .collect()
    };
    MessageAttachment {
        name: attachment.name.clone(),
        media_type: attachment.mime_type.clone(),
        kind,
        pages,
        text_content: attachment.text_content.clone().filter(|text| !text.is_empty()),
        extracted_text: attachment.extracted_text.clone().filter(|text| !text.is_empty()),
        page_count: attachment.page_count.filter(|count| *count > 0),
    }
}

/// Messages of the turn: the history the agent sees, each message with its
/// own files, and the new message with the files of this turn. A file sent
/// earlier stays where it was answered: on the new message it looked freshly
/// sent, and the tools that use the message's photo took it again.
pub fn turn_messages(history: &[StoredChatMessage], prompt: &str, attachments: &[StoredChatAttachment]) -> Vec<BackendMessage> {
    let mut messages = history
        .iter()
        .map(|message| BackendMessage {
            role: match message.role {
                ChatRole::User => MessageRole::User,
                ChatRole::Assistant => MessageRole::Assistant,
            },
            content: message.content.clone(),
            images: Vec::new(),
            attachments: message.attachments.iter().map(message_attachment).collect(),
        })
        .collect::<Vec<_>>();
    messages.push(BackendMessage {
        role: MessageRole::User,
        content: prompt.to_string(),
        images: Vec::new(),
        attachments: attachments.iter().map(message_attachment).collect(),
    });
    messages
}

/// Scope of a label the interface uses (`task-manager`, `published-task-manager`, `graph-view`…).
pub fn scope_of(label: &str) -> BackendScope {
    if label.contains("finance") {
        BackendScope::Finance
    } else if label.contains("task") || label == "published" {
        // The published boards are a Task Manager, as their route says.
        BackendScope::TaskManager
    } else if label.contains("graph") {
        BackendScope::Graph
    } else if label.contains("document") {
        BackendScope::Document
    } else {
        BackendScope::Library
    }
}

/// Memory policy of a chat turn: a chat created without agent memory runs
/// without it, so the engine neither injects `memory.md` nor offers its
/// memory tools. Other routes keep their policy.
pub fn chat_persistence_policy(route: PersistencePolicy, document: Option<&StoredChatDocument>) -> PersistencePolicy {
    let without_memory = document.is_some_and(|document| !document.agent_memory_enabled);
    if route == PersistencePolicy::Persistent && without_memory {
        PersistencePolicy::EphemeralNoMemory
    } else {
        route
    }
}

/// Channel, scope and memory policy of a turn.
pub fn turn_route(mode: TurnMode, scope_label: &str, view: Option<&str>) -> (BackendChannel, BackendScope, PersistencePolicy) {
    match mode {
        TurnMode::Meeting => (BackendChannel::Meeting, BackendScope::Library, PersistencePolicy::EphemeralNoMemory),
        TurnMode::Published => (BackendChannel::Published, BackendScope::TaskManager, PersistencePolicy::PublishedNoMemory),
        TurnMode::Chat => {
            let scope = scope_of(scope_label);
            // A library chat opened over Meeting talks on the Meeting channel.
            let channel = if scope == BackendScope::Library && view == Some("meeting") {
                BackendChannel::Meeting
            } else {
                BackendChannel::App
            };
            (channel, scope, PersistencePolicy::Persistent)
        }
    }
}

/// What the agent may use of the visible workspace, by the scope the
/// interface shows. It describes the context; authorization stays in the
/// runtime.
pub fn snapshot_capabilities(scope_label: &str) -> SnapshotCapabilities {
    let document = scope_label == "document";
    let library_or_graph = scope_label == "library" || scope_label == "graph";
    SnapshotCapabilities {
        can_read_active_document: document,
        can_read_library: library_or_graph || document || scope_label == "published",
        can_write_active_document: document,
        can_write_library: library_or_graph,
        can_search_web: scope_label != "finance" && scope_label != "published",
        can_ask_clarification: true,
        can_request_confirmation: true,
    }
}

/// A document of the workspace as the interface describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDocumentInput {
    pub path: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub dirty: bool,
}

/// The visible workspace when the person sent the message.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInput {
    #[serde(default = "default_snapshot_version")]
    pub snapshot_version: u16,
    pub view: String,
    pub scope: String,
    #[serde(default)]
    pub active_document: Option<WorkspaceDocumentInput>,
    #[serde(default)]
    pub open_tabs: Vec<WorkspaceDocumentInput>,
    #[serde(default)]
    pub selection: Option<SelectionSnapshot>,
    #[serde(default)]
    pub captured_at: u64,
}

fn default_snapshot_version() -> u16 {
    1
}

fn document_snapshot(document: &WorkspaceDocumentInput) -> DocumentSnapshot {
    DocumentSnapshot {
        path: document.path.clone(),
        name: document.name.clone(),
        kind: document.kind.clone(),
        revision: document.revision,
        dirty: document.dirty,
    }
}

/// Snapshot of the workspace for the agent. A selection is kept only when it
/// belongs to the active document.
pub fn workspace_snapshot(input: &WorkspaceInput, library_id: &str) -> BackendSnapshot {
    let active_path = input.active_document.as_ref().map(|document| document.path.as_str());
    BackendSnapshot {
        snapshot_version: input.snapshot_version,
        view: input.view.clone(),
        scope: scope_of(&input.scope),
        library_id: library_id.to_string(),
        active_document: input.active_document.as_ref().map(document_snapshot),
        open_tabs: input.open_tabs.iter().map(document_snapshot).collect(),
        capabilities: snapshot_capabilities(&input.scope),
        captured_at: input.captured_at,
        selection: input
            .selection
            .clone()
            .filter(|selection| Some(selection.document_path.as_str()) == active_path),
    }
}

/// Context a view asks its chat to keep: the scope key and, optionally, a
/// mode with the files of the view (logical paths).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewContext {
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub mode: Option<ChatContextMode>,
    #[serde(default)]
    pub files: Vec<String>,
}

/// A path as chats compare it: forward slashes, no surrounding slashes.
pub fn comparable_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_matches('/').to_string()
}

fn comparable_set(paths: &[String]) -> Vec<String> {
    let mut paths = paths.iter().map(|path| comparable_path(path)).filter(|path| !path.is_empty()).collect::<Vec<_>>();
    paths.sort();
    paths
}

/// Folder of a Task Manager board (`…/task-manager/<board>/`), from the board
/// scope key and one file of the board.
pub fn board_prefix(context: &ViewContext) -> Option<String> {
    let board = context.scope_key.as_deref()?.strip_prefix("task-manager:board:")?.trim().to_lowercase();
    let path = comparable_path(context.files.first()?);
    if board.is_empty() {
        return None;
    }
    let (index, marker) = ["task-manager/", "task-mannager/"]
        .iter()
        .find_map(|marker| path.find(marker).map(|index| (index, *marker)))?;
    Some(format!("{}{marker}{board}/", &path[..index]))
}

/// How well a chat fits the context of a view: 3 same scope, 2 same mode
/// and files, 1 files of the same board, 0 unrelated.
pub fn context_match_score(document: &StoredChatDocument, context: &ViewContext, board_prefix: Option<&str>) -> u8 {
    let same_scope = context.scope_key.is_some() && document.context_scope_key == context.scope_key;
    let same_mode = context.mode == Some(document.selected_context_mode);
    let files = comparable_set(&document.selected_context_files);
    let same_files = same_mode && !context.files.is_empty() && files == comparable_set(&context.files);
    let same_board = same_mode
        && !files.is_empty()
        && board_prefix.is_some_and(|prefix| files.iter().all(|file| file.starts_with(prefix)));
    if same_scope {
        3
    } else if same_files {
        2
    } else if same_board {
        1
    } else {
        0
    }
}

/// The best scored chat; the selected one wins a tie.
pub fn best_match(scores: &[(String, u8)], selected: Option<&str>) -> Option<String> {
    let best = scores.iter().map(|(_, score)| *score).max().filter(|score| *score > 0)?;
    scores
        .iter()
        .find(|(path, score)| *score == best && Some(path.as_str()) == selected)
        .or_else(|| scores.iter().find(|(_, score)| *score == best))
        .map(|(path, _)| path.clone())
}

/// A chat opened from a view keeps that view's scope and, when the view has
/// files, its mode and files. Returns whether the chat changed.
pub fn apply_view_context(document: &mut StoredChatDocument, context: &ViewContext) -> bool {
    let before = (document.context_scope_key.clone(), document.selected_context_mode, document.selected_context_files.clone());
    document.context_scope_key = context.scope_key.clone();
    if let Some(mode) = context.mode {
        document.selected_context_mode = mode;
        if !context.files.is_empty() {
            document.selected_context_files = context.files.clone();
        }
    }
    before != (document.context_scope_key.clone(), document.selected_context_mode, document.selected_context_files.clone())
}

/// Answer of a turn that undid an AI change.
pub fn undo_answer(path: Option<&str>) -> String {
    match path.filter(|path| !path.is_empty()) {
        Some(path) => format!("Deshice el último cambio de IA en `{path}`."),
        None => "Deshice el último cambio de IA.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_with_only_files_asks_the_agent_to_decide() {
        assert_eq!(turn_request("  cargá este ticket ", 1).as_deref(), Some("cargá este ticket"));
        assert_eq!(turn_request("", 2).as_deref(), Some(ATTACHMENTS_ONLY_REQUEST));
        assert_eq!(turn_request("   ", 0), None);
        assert_eq!(title_seed("", &["ticket-coto.jpg", "resumen visa.pdf"]), "ticket-coto, resumen visa");
        assert_eq!(title_seed("Resumen de gastos", &["a.png"]), "Resumen de gastos");
        assert_eq!(title_from_prompt(&title_seed("", &["ticket-coto.jpg"])), "ticket-coto");
    }

    fn message(role: ChatRole, content: &str) -> StoredChatMessage {
        StoredChatMessage { role, content: content.into(), attachments: Vec::new(), agent: None }
    }

    #[test]
    fn chat_settings_decide_tools_and_the_permanent_context() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        assert_eq!(chat_tool_access(Some(&document)), ToolAccess::All);
        assert_eq!(chat_tool_access(None), ToolAccess::All);
        assert_eq!(permanent_context_block(Some(&document)), None);
        let settings = ChatSettings {
            tools_enabled: true,
            write_enabled: false,
            permanent_context: "  Respondé corto. ".into(),
            dynamic: Some(" ".into()),
            agents: vec![" epicteto.md ".into()],
        };
        apply_chat_settings(&mut document, &settings).expect("settings");
        assert_eq!(chat_tool_access(Some(&document)), ToolAccess::ReadOnly);
        assert_eq!(document.dynamic, None);
        assert_eq!(document.agents, vec!["epicteto.md".to_string()]);
        assert!(permanent_context_block(Some(&document)).unwrap().ends_with("sin permisos):
Respondé corto."));
        apply_chat_settings(&mut document, &ChatSettings { tools_enabled: false, ..settings.clone() }).expect("settings");
        assert_eq!(chat_tool_access(Some(&document)), ToolAccess::None);
        // Invalid settings leave the chat as it was.
        let invalid = ChatSettings { agents: vec!["../x.md".into()], ..settings };
        assert!(apply_chat_settings(&mut document, &invalid).is_err());
        assert_eq!(document.agents, vec!["epicteto.md".to_string()]);
    }

    fn attachment(kind: ChatAttachmentKind) -> StoredChatAttachment {
        StoredChatAttachment {
            name: "a".into(),
            mime_type: "image/png".into(),
            base64: "AAA".into(),
            additional_base64: vec!["BBB".into()],
            kind,
            extracted_text: None,
            text_content: Some("texto".into()),
            page_count: None,
        }
    }

    #[test]
    fn the_memory_window_keeps_the_last_messages() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 2);
        document.messages = vec![message(ChatRole::User, "1"), message(ChatRole::Assistant, "2"), message(ChatRole::User, "3")];
        assert_eq!(memory_window(&document).iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["2", "3"]);
        document.context_memory_enabled = false;
        assert!(memory_window(&document).is_empty());
    }

    /// «Reintentar» takes the last request out with the replies after it,
    /// so the turn runs again on the conversation before it.
    #[test]
    fn the_last_request_is_taken_out_to_be_answered_again() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        document.messages = vec![
            message(ChatRole::User, "hola"),
            message(ChatRole::Assistant, "¡Hola!"),
            message(ChatRole::User, "¿qué puede hacer este módulo?"),
            message(ChatRole::Assistant, "article 4 hola"),
            message(ChatRole::Assistant, "otra nota del mismo turno"),
        ];
        let request = take_last_request(&mut document).expect("request");
        assert_eq!(request.content, "¿qué puede hacer este módulo?");
        assert_eq!(document.messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["hola", "¡Hola!"]);
        // A request without an answer yet (the turn failed) is taken as well.
        document.messages.push(message(ChatRole::User, "otra vez"));
        assert_eq!(take_last_request(&mut document).map(|m| m.content), Some("otra vez".to_string()));
        let mut empty = StoredChatDocument::new("Chat".into(), true, true, 10);
        empty.messages = vec![message(ChatRole::Assistant, "Bienvenido")];
        assert_eq!(take_last_request(&mut empty), None);
        assert_eq!(empty.messages.len(), 1);
    }

    #[test]
    fn new_chats_are_named_after_the_first_sentence_that_is_not_a_greeting() {
        assert_eq!(title_from_prompt("Hola. ¿Cómo armo un presupuesto mensual para la casa con ahorro?"), "¿Cómo armo un presupuesto mensual para la casa");
        assert_eq!(title_from_prompt("  "), UNTITLED_CHAT_TITLE);
        assert_eq!(title_from_prompt("hola"), "hola");
        assert_eq!(persisted_title("Chat", false, "Plan de viaje"), "Plan de viaje");
        assert_eq!(persisted_title("Chat temporal 3", false, "Plan"), "Plan");
        assert_eq!(persisted_title("Mi chat", false, "Plan"), "Mi chat");
        assert_eq!(persisted_title("Chat", true, "Plan"), "Chat");
    }

    #[test]
    fn new_chats_take_the_composer_context_but_not_a_temporary_one() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        let selection = ContextSelection {
            scope_key: Some("board".into()),
            files: vec!["a.md".into()],
            mode: ChatContextMode::Index,
            folders: vec!["notas".into()],
            library_rag: false,
            keep_chat_context: false,
        };
        prepare_new_chat(&mut document, &selection);
        assert_eq!(document.selected_context_mode, ChatContextMode::Index);
        assert_eq!(document.selected_context_files, ["a.md"]);
        assert_eq!(document.selected_context_folders, ["notas"]);
        assert!(!document.library_rag_enabled);
        let temporary = ContextSelection { files: vec!["b.md".into()], keep_chat_context: true, ..Default::default() };
        apply_turn_context(&mut document, &temporary);
        assert_eq!(document.selected_context_files, ["a.md"]);
        assert_eq!(document.context_scope_key.as_deref(), Some("board"));
    }

    #[test]
    fn each_message_keeps_its_attachments_and_the_turn_carries_the_context() {
        let mut history = vec![message(ChatRole::User, "antes")];
        history[0].attachments.push(attachment(ChatAttachmentKind::Image));
        let messages = turn_messages(&history, "ahora", &[attachment(ChatAttachmentKind::Text)]);
        assert_eq!(messages.len(), 2);
        // The earlier photo stays on its message; the turn has only its file.
        assert_eq!(messages[0].attachments.len(), 1);
        assert_eq!(messages[0].attachments[0].pages, ["AAA", "BBB"]);
        assert_eq!(messages[1].attachments.len(), 1);
        assert!(messages[1].attachments[0].pages.is_empty());
        assert!(turn_prompt(TurnMode::Chat, "hola", Some(" sala ")).ends_with("\nsala"));
        assert!(turn_prompt(TurnMode::Meeting, "¿qué dijo?", Some("texto")).contains("TRANSCRIPCIÓN ACTUAL:\ntexto"));
    }

    #[test]
    fn chats_are_matched_by_scope_files_or_board() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        document.selected_context_mode = ChatContextMode::Index;
        document.selected_context_files = vec!["task-mannager/equipo/b.md".into()];
        let context = ViewContext {
            scope_key: Some("task-manager:board:Equipo".into()),
            mode: Some(ChatContextMode::Index),
            files: vec!["\\task-mannager\\equipo\\a.md".into()],
        };
        let prefix = board_prefix(&context);
        assert_eq!(prefix.as_deref(), Some("task-mannager/equipo/"));
        assert_eq!(context_match_score(&document, &context, prefix.as_deref()), 1);
        document.selected_context_files = vec!["task-mannager/equipo/a.md".into()];
        assert_eq!(context_match_score(&document, &context, prefix.as_deref()), 2);
        document.context_scope_key = context.scope_key.clone();
        assert_eq!(context_match_score(&document, &context, prefix.as_deref()), 3);
        let scores = vec![("a".to_string(), 2), ("b".to_string(), 2), ("c".to_string(), 0)];
        assert_eq!(best_match(&scores, Some("b")).as_deref(), Some("b"));
        assert_eq!(best_match(&scores, None).as_deref(), Some("a"));
        assert_eq!(best_match(&[("c".to_string(), 0)], None), None);
    }

    #[test]
    fn a_view_context_replaces_the_scope_and_its_files() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        document.selected_context_files = vec!["x.md".into()];
        let scope_only = ViewContext { scope_key: Some("graph".into()), ..Default::default() };
        assert!(apply_view_context(&mut document, &scope_only));
        assert_eq!(document.selected_context_files, ["x.md"]);
        assert!(!apply_view_context(&mut document, &scope_only));
        let with_files = ViewContext { scope_key: None, mode: Some(ChatContextMode::Index), files: vec!["y.md".into()] };
        assert!(apply_view_context(&mut document, &with_files));
        assert_eq!((document.selected_context_mode, document.selected_context_files.clone()), (ChatContextMode::Index, vec!["y.md".to_string()]));
    }

    #[test]
    fn a_chat_without_agent_memory_runs_without_memory() {
        let mut document = StoredChatDocument::new("Chat".into(), true, true, 10);
        assert_eq!(chat_persistence_policy(PersistencePolicy::Persistent, Some(&document)), PersistencePolicy::Persistent);
        document.agent_memory_enabled = false;
        assert_eq!(chat_persistence_policy(PersistencePolicy::Persistent, Some(&document)), PersistencePolicy::EphemeralNoMemory);
        assert_eq!(chat_persistence_policy(PersistencePolicy::Persistent, None), PersistencePolicy::Persistent);
        assert_eq!(chat_persistence_policy(PersistencePolicy::PublishedNoMemory, Some(&document)), PersistencePolicy::PublishedNoMemory);
    }

    #[test]
    fn routes_and_snapshots_follow_the_visible_scope() {
        assert_eq!(turn_route(TurnMode::Chat, "library", Some("meeting")).0, BackendChannel::Meeting);
        assert_eq!(turn_route(TurnMode::Chat, "document", Some("meeting")).0, BackendChannel::App);
        assert_eq!(turn_route(TurnMode::Published, "", None).2, PersistencePolicy::PublishedNoMemory);
        let input = WorkspaceInput {
            snapshot_version: 1,
            view: "editor".into(),
            scope: "document".into(),
            active_document: Some(WorkspaceDocumentInput { path: "a.md".into(), name: "a".into(), kind: "markdown".into(), revision: 3, dirty: false }),
            open_tabs: Vec::new(),
            selection: Some(SelectionSnapshot { document_path: "b.md".into(), from: 0, to: 1, blocks: Vec::new() }),
            captured_at: 1,
        };
        let snapshot = workspace_snapshot(&input, "lib");
        assert_eq!(snapshot.scope, BackendScope::Document);
        assert!(snapshot.capabilities.can_write_active_document && !snapshot.capabilities.can_write_library);
        assert!(snapshot.selection.is_none());
    }
}
