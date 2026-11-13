//! Core de ejecución independiente de Tauri y del WebView.
//!
//! Este módulo contiene únicamente contratos, estado de request y puertos del
//! backend. Los adaptadores de Tauri, filesystem, SQLite, Telegram y modelos
//! se mantienen fuera del core para que el mismo runtime pueda ejecutarse sin
//! una ventana.

pub mod agent;
pub mod agent_autonomy;
pub mod agent_knowledge;
pub mod agent_scripts;
pub mod agent_workspace;
pub mod ai_actions;
pub mod ai_tasks;
pub mod ai_settings;
pub mod auto_confirm;
pub mod audio_devices;
pub mod audio_resample;
pub mod browser;
pub mod speaker_clustering;
pub mod timed_transcript;
pub mod catalog;
pub mod chat_agents;
pub mod chat_attachments;
pub mod chat_context;
pub mod chat_history;
pub mod chat_list;
pub mod chat_turn;
pub mod coldpass;
pub mod config_envelope;
pub mod connection;
pub mod mirror_sync;
pub mod context;
pub mod continuation;
pub mod device_preferences;
pub mod control;
pub mod error;
pub mod events;
pub mod export;
pub mod export_render;
pub mod finance_answer;
pub mod folder_contexts;
pub mod finance_insights;
pub mod formatting;
pub mod gitbook_blocks;
pub mod interaction;
pub mod isolation;
pub mod library_config;
pub mod library_graph;
pub mod library_tree;
pub mod pomodoro;
pub mod pomodoro_log;
pub mod library_tools;
pub mod mail_accounts;
pub mod mail_tools;
pub mod markdown_editing;
pub mod meeting;
pub mod meeting_ai;
pub mod meeting_review;
pub mod meeting_tools;
pub mod mermaid;
pub mod note_diagram;
pub mod note_ink;
pub mod note_ink_conversion;
pub mod note_preview;
pub mod page_links;
pub mod page_setup;
pub mod paths;
pub mod ports;
pub mod prompt;
pub mod public_spaces;
pub mod prompt_guidance;
pub mod gym;
pub mod image_generation;
pub mod health;
pub mod recipes;
pub mod protocol;
pub mod remote_audio;
pub mod reply_wait;
pub mod runtime;
pub mod speech_text;
pub mod task_board_actions;
pub mod task_board_plan;
pub mod task_board_tool_input;
pub mod task_detail_view;
pub mod task_management_view;
pub mod task_planner;
pub mod task_manager_tool_input;
pub mod task_manager_tools;
pub mod task_manager_ui;
pub mod telegram_bot;
pub mod terminal;
pub mod text_merge;
pub mod text_search;
pub mod tool_call_recovery;
pub mod tool_routing;
pub mod turn_interrupts;
pub mod weather;
pub mod web_search;
pub mod webhooks;
pub mod whatsapp;
pub mod wiki_links;

pub use agent::{
    contains_pending_action, run_agent, run_agent_with_interaction, run_agent_with_resume,
    tool_call_key, AgentContinuation, AgentProvider, AgentRunResult, AgentRuntimeOptions,
    AgentStateHooks, NoopAgentState, ProviderMessage, ProviderMessageRole, ProviderRequest,
    ProviderResponse, ProviderStreamDelta, ProviderToolCall, ToolExecutor,
};
pub use catalog::{
    authorize_tool_call, canonical_tool_catalog, project_canonical_tool_catalog,
    project_tool_catalog, restrict_tool_access, tool_policy, AuthorizationPrincipal, ToolCatalogProjection,
    ToolPolicy,
};
pub use paths::{
    authorize_agent_path, authorize_agent_path_for, authorize_library_path,
    normalize_prompt_file_name, validate_agent_path, validate_logical_library_path, AgentPath,
    AgentPathKind, LogicalLibraryPath, ScopedAgentPath, ScopedLibraryPath, AGENT_DIRECTORY,
    DEFAULT_PROMPT_FILE, MEMORY_DIRECTORY, MEMORY_FILE, PROMPTS_DIRECTORY, RULES_FILE,
    SKILLS_DIRECTORY, THOUGHTS_FILE, BIOGRAPHY_FILE, TALK_FILE, PEOPLE_FILE, IMPROVE_FILE,
};
pub use ports::{
    load_biography_for_context, load_memory_for_context, load_people_for_context, load_prompt_for_context, load_rules_for_context,
    load_skills_for_context, load_talk_for_context, load_thoughts_for_context, save_memory_for_context, synchronize_default_prompt_for_context,
    AgentFileRepository, AgentSkill, AgentStateRepository, DocumentRepository, LibraryDescriptor,
    LibraryRepository, ScopedDocumentRepository,
};
pub use prompt::{
    compose_system_prompt, load_prompt_parts, load_prompt_parts_with_request,
    resolve_rules_for_format, response_format_for_channel, strip_frontmatter, PromptLoadRequest,
    PromptParts, base_agent_prompt, CONVERSATION_AGENT_PROMPT, DEFAULT_AGENT_PROMPT, DEFAULT_AGENT_RULES,
};
pub use web_search::{
    normalize_http_url, sanitize_web_search_request, validate_citations, web_search_key,
    WebSearchRequest, WebSearchReservation, WebSearchTracker, MAX_UNIQUE_WEB_SEARCHES,
};

pub use context::{
    BackendActor, BackendChannel, BackendRequestContext, BackendScope, PersistencePolicy,
    OWNER_LIBRARY_USER_ID,
};
pub use control::{RequestControl, RequestControlRegistry, MAX_ACTIVE_REQUEST_CONTROLS};
pub use error::{BackendError, BackendErrorCode};
pub use events::{BackendEvent, BackendEventSink, NoopEventSink, VecEventSink};
pub use export::{
    export_destination_path, export_fingerprint, preview_bounded_export, MAX_EXPORT_NAME_ATTEMPTS, preview_export_with_port,
    validate_export_input_size, validate_export_request, write_export_with_port, BoundedExportPort,
    BoundedExportRequest, ExportFormat, ExportPreview, ExportReceipt, ExportRecovery,
    ExportRecoveryState, RecoverableExportResult, MAX_EXPORT_INPUT_BYTES, MAX_EXPORT_OUTPUT_BYTES,
};
pub use export_render::{
    diagram_sources, render_markdown_export, render_markdown_export_with_diagrams, DiagramKind, DiagramRenderInput, DiagramRenders,
    DiagramSource,
};
pub use page_setup::PageGeometry;
pub use formatting::{channel_response, escape_telegram_html, markdown_to_telegram_html};
pub use interaction::{
    begin_operation, ensure_preview_revisions, transition_operation_state,
    validate_clarification_answer, validate_confirmation, ClarificationAnswer, ClarificationOption,
    ClarificationRequest, ConfirmationRequest, ConfirmationSelection, ExecutionPlan,
    OperationGenerationCache, OperationLease, OperationReview, OperationReviewPort,
    OperationStatePort, OperationToken, PlanStatus, PlanStep, PlanStepStatus, RevisionPort,
    UndoRequest, UndoResult,
};
pub use isolation::{GenerationCache, InFlightRegistry, ScopedKey, TenantStateRegistry};
pub use library_tools::{
    compute_document_revision, library_mutation_tool_contracts, library_tool_contracts,
    validate_android_document_uri, validate_android_tree_uri, validate_document_locator,
    validate_logical_path, AndroidDocumentUriDto, AndroidTreeUriDto, AtomicDocumentWritePort,
    AtomicWriteRequest, BoundedPage, CompareDocumentsRequest, ContextSearchRequest,
    DocumentComparison, DocumentContent, DocumentIndexCache, DocumentKind, DocumentLocatorDto,
    DocumentMetadata, DocumentReferences, DocumentRevisionPort, DocumentSearchHit,
    DocumentSearchRequest, DocumentWritePreview, ExactSearchRequest, InMemoryLibrary,
    InventoryRequest, LibraryDescriptorDto, LibraryDocumentPort, LibraryDocumentReadPort,
    LibraryIndexPort, LineDifference, LogicalPathDto, PreviewReplaceRequest,
    RecoverableAtomicWritePort, RecoveryState, ReferenceMatch, ReindexRequest, ReindexResult,
    RevisionCheck, RevisionRequest, SearchFragment, SeedDocument, WriteReceipt, MAX_COMPARE_LINES,
    MAX_DOCUMENT_CHARS, MAX_INVENTORY_ITEMS, MAX_LOGICAL_PATH_CHARS, MAX_PATH_SEGMENT_CHARS,
    MAX_READ_DOCUMENTS, MAX_READ_DOCUMENT_CHARS, MAX_REFERENCE_RESULTS, MAX_SEARCH_CANDIDATES,
    MAX_SEARCH_RESULTS,
};
pub use markdown_editing::{
    apply_markdown_preview, apply_multi_document_markdown_preview, ensure_markdown_defaults,
    note_page_mode, preview_markdown_edit, set_note_page_mode, PAGE_MODE_PROPERTY,
    materialize_markdown_preview, preview_markdown_edit_from_port,
    preview_multi_document_markdown_edit,
    validate_markdown_format, MarkdownAnchor, MarkdownAnchorKind, MarkdownApplyResult,
    MarkdownDocumentEdit, MarkdownEditPreview, MarkdownEditRequest, MarkdownFormatValidation,
    MarkdownHunk, MarkdownOperation, MarkdownPreviewDocument, MultiDocumentMarkdownApplyResult,
    MultiDocumentMarkdownEditRequest, MultiDocumentMarkdownPreview, SemanticMarkdownPreview,
    MAX_MARKDOWN_HUNKS, MAX_MARKDOWN_TAG_CHARS, MAX_MULTI_DOCUMENT_DOCUMENTS, MAX_MULTI_DOCUMENT_EDITS,
};
pub use prompt_guidance::{scope_guidance, XGRAPH_AGENT_GUIDE};
pub use protocol::{
    AgentRequest, AgentResponse, AttachmentKind, AttachmentRef, BackendEventEnvelope,
    BackendLimits, BackendMessage, BackendRequest, BackendRequestEnvelope, BackendResponse,
    BackendResponseEnvelope, BackendSnapshot, CancelRequest, ChannelResponse, ConfirmationDecision,
    DocumentSnapshot, GetOperationRequest, MessageRole, MutationPreview, MutationPreviewAction,
    OperationState, OperationStatus, PendingInteraction, PlanDecision, PreviewDocument,
    PreviewHunk, ProtocolVersion, ResumeDecision, ResumeRequest, ReviewRequest,
    SelectionBlockSnapshot, SelectionSnapshot,
    SnapshotCapabilities, ToolAccess, ToolCall, ToolDefinition, ToolResult, UndoOperationRequest,
    MAX_BACKEND_PROTOCOL_VERSION, MAX_TOOL_POOL,
};
pub use runtime::InteractionRuntime;
pub use task_manager_tool_input::{
    is_task_mutation_tool, read_tickets_by_id, task_mutation_from_tool, ticket_ids_to_read, MAX_TASK_TOOL_READ_TICKETS,
    TASK_MUTATION_TOOLS,
};
pub use task_manager_ui::{
    plan_pomodoro_record, pomodoro_hours_update, pomodoro_ticket, project_board_view, show_board_paths,
    resolve_board_intent, PomodoroDurationsDto, PomodoroEvent, PomodoroRecordPlan,
    TaskBoardIntent, TaskBoardViewDto,
};
pub use task_manager_tools::{
    task_manager_mutation_tool_contracts, task_manager_read_tool_contracts, ticket_detail_preview,
    InMemoryTaskManager,
    PersistentTaskManager, TaskBoardDto, TaskBoardSummaryDto, TaskCommentDto, TaskContextHitDto,
    TaskContextSearchRequest, TaskDocumentRouteDto, TaskEntityRevisionDto, TaskGroupDto,
    TaskManagerAppliedOperationDto, TaskManagerConfigDto, TaskManagerContextDto,
    TaskManagerLibrarySnapshotDto, TaskManagerListRequest, TaskManagerMutationPort,
    TaskManagerOptionsDto, TaskManagerReadPort, TaskManagerSnapshotDto, TaskManagerSnapshotReadDto,
    TaskManagerSnapshotStore, TaskManagerStoreCommit, TaskMutationApplyRequestDto, TaskMutationDto,
    TaskMutationPreviewDto, TaskMutationReceiptDto, TaskMutationRequestDto, TaskPriority,
    TaskState, TaskTicketDto, TaskTicketListRequest, TaskTicketReadDto, TaskTicketReadRequest,
    TaskTicketSummaryDto, TaskUpdateFieldsDto, MAX_TASK_BOARDS, MAX_TASK_BULK_TICKETS,
    MAX_TASK_DATE_CHARS, MAX_TASK_GROUPS, MAX_TASK_HOURS, MAX_TASK_LIBRARIES,
    MAX_TASK_LIBRARY_COMMENTS, MAX_TASK_ORDER, MAX_TASK_REFERENCE_CHARS, MAX_TASK_RELATED_REFERENCES, MAX_TASK_RESULTS,
    MAX_TASK_SNAPSHOT_BYTES, MAX_TASK_TEXT_CHARS, MAX_TASK_TICKETS, MAX_TASK_USERS,
    TASK_MANAGER_SNAPSHOT_READ_VERSION, TASK_MANAGER_SNAPSHOT_VERSION,
};
