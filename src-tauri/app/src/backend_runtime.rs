//! Productive Tauri wiring for the backend-core agent runtime.
//!
//! This module is deliberately an adapter: authorization, interaction state
//! transitions and the provider loop remain in `backend-core`. Tauri only
//! resolves the native principal, supplies platform-bound document reads and
//! publishes versioned events.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use notia_backend_core::{
    AgentContinuation, AgentProvider, AgentResponse, AgentStateHooks, BackendError, BackendErrorCode,
    BackendLimits, BackendRequest, BackendRequestContext, BackendSnapshot,
    AgentRequest, BackendRequestEnvelope, BackendResponse, BackendResponseEnvelope, BackendScope,
    DocumentContent, DocumentLocatorDto, DocumentSearchRequest, ExactSearchRequest,
    CompareDocumentsRequest, ContextSearchRequest, LibraryDocumentReadPort, MutationPreview,
    MutationPreviewAction,
    OperationGenerationCache, PreviewDocument, PreviewHunk,
    OperationReview, OperationReviewPort, OperationStatePort, OperationToken, PersistencePolicy,
    ProviderMessage, ProviderMessageRole, ProviderRequest, ProviderResponse, ProviderStreamDelta,
    RequestControl, RequestControlRegistry, RevisionPort, ToolCall, ToolExecutor, ToolResult, UndoRequest, UndoResult,
    AgentRuntimeOptions, InteractionRuntime, ToolCatalogProjection,
    project_canonical_tool_catalog, ResumeDecision, MarkdownEditPreview, MarkdownEditRequest,
    MultiDocumentMarkdownEditRequest, MultiDocumentMarkdownPreview, materialize_markdown_preview, preview_markdown_edit,
    preview_multi_document_markdown_edit, WebSearchReservation, WebSearchTracker,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::host::{AppHandle, Manager};

use crate::finance_agent_inputs;
use crate::backend_ollama::{platform_ollama_transport, OllamaAgentProvider, OllamaProviderConfig};
use crate::backend_tauri::{
    BackendEventStore as TauriBackendEventStore, EventStreamKey, TauriBackendEventSink,
};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};
use crate::library_users::backend_authorization_principal;
use crate::mobile_directory_picker::AndroidDirectoryPickerState;

/// Host-owned runtime state. Every field is shared so a request can run on a
/// blocking worker while other transport calls (cancel, status, replay) are
/// served concurrently against the same journal, controls and events.
#[derive(Debug, Default, Clone)]
pub(crate) struct BackendRuntimeState {
    journal: Arc<BackendJournal>,
    generations: Arc<OperationGenerationCache>,
    events: Arc<TauriBackendEventStore>,
    controls: Arc<RequestControlRegistry>,
    provider: Arc<Mutex<Option<BackendProviderSettings>>>,
}

impl BackendRuntimeState {
    /// Uses the library's saved AI settings (`ia` section of its
    /// configuration), so a backend worker can run without the WebView.
    pub(crate) fn configure_from_library_config(&self, config: &Value) -> Result<(), BackendError> {
        let settings = provider_settings_from_config(config)?;
        *self
            .provider
            .lock()
            .map_err(|_| internal_error("No se pudo configurar el proveedor backend."))? = Some(settings);
        Ok(())
    }

    /// Provider settings the interface is using; a library without saved AI
    /// settings runs with them.
    pub(crate) fn configure_fallback(
        &self,
        settings: &notia_backend_core::ai_settings::AiSettings,
    ) -> Result<(), BackendError> {
        if settings.selected_model.is_empty() {
            return Ok(());
        }
        let think = if settings.thinking_enabled {
            Value::String(settings.thinking_level.as_str().to_string())
        } else {
            Value::Bool(false)
        };
        *self
            .provider
            .lock()
            .map_err(|_| internal_error("No se pudo configurar el proveedor backend."))? =
            Some(BackendProviderSettings {
                ollama_url: settings.ollama_url.clone(),
                model: settings.selected_model.clone(),
                api_key: settings.api_key.clone(),
                think,
            });
        Ok(())
    }

    /// Events of one request after `after_sequence`, for a backend observer.
    pub(crate) fn request_events(
        &self,
        context: &notia_backend_core::BackendRequestContext,
        after_sequence: u64,
    ) -> Result<Vec<notia_backend_core::BackendEventEnvelope>, BackendError> {
        self.events.replay_since(
            &EventStreamKey::from_parts(&context.library_id, &context.actor.library_user_id, &context.request_id),
            after_sequence,
        )
    }

    fn interactions<'a>(
        &'a self,
        revisions: Option<&'a dyn RevisionPort>,
    ) -> InteractionRuntime<'a> {
        InteractionRuntime::new(
            self.journal.as_ref(),
            Some(self.journal.as_ref()),
            revisions,
            self.generations.as_ref(),
        )
        .with_control_registry(Arc::clone(&self.controls))
    }

    /// Cancels every active run of a library whose binding was revoked.
    pub(crate) fn cancel_library_runs(&self, library_id: &str) -> Result<usize, BackendError> {
        self.controls.cancel_library(library_id)
    }
}

#[derive(Debug, Clone)]
struct BackendProviderSettings {
    ollama_url: String,
    model: String,
    api_key: String,
    think: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct JournalKey {
    library_id: String,
    library_user_id: String,
    idempotency_key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct JournalRecord {
    context: Option<BackendRequestContext>,
    request: Option<AgentRequest>,
    operation: Option<OperationReview>,
    response: Option<AgentResponse>,
    tool_results: HashMap<String, ToolResult>,
    continuation: Option<AgentContinuation>,
    /// Pre-mutation state of documents changed by confirmed operations,
    /// keyed by operation id. Enables a verified undo of the latest change.
    #[serde(default)]
    undo: HashMap<String, DocumentUndoEntry>,
}

impl JournalRecord {
    /// The record as saved: images and attached files stay in memory for
    /// the run and are not written to the journal (the text of the turn,
    /// its tool results and its continuation are).
    fn without_binary_content(mut self) -> Self {
        if let Some(request) = self.request.as_mut() {
            for message in &mut request.messages {
                message.images.clear();
                message.attachments.clear();
            }
        }
        if let Some(continuation) = self.continuation.as_mut() {
            for message in &mut continuation.messages {
                message.images.clear();
            }
        }
        self
    }
}

/// State needed to revert one document mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentUndoEntry {
    logical_path: String,
    /// Content before the mutation; `None` when the operation created it.
    previous_content: Option<String>,
    /// Revision right after the mutation; `None` when it deleted the file.
    resulting_revision: Option<u64>,
}

/// Documents larger than this are not kept for undo; the operation simply
/// stays non-undoable instead of bloating the journal.
const MAX_UNDO_CONTENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default)]
struct BackendJournal {
    records: Mutex<HashMap<JournalKey, JournalRecord>>,
}

const MAX_JOURNAL_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_JOURNAL_RECORDS: usize = 256;

impl BackendJournal {
    /// Stores the undo entry of a completed document mutation and marks the
    /// matching operation as undoable.
    fn store_undo(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        operation_id: &str,
        entry: DocumentUndoEntry,
    ) -> Result<(), BackendError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(context, idempotency_key);
        let record = records.entry(key.clone()).or_default();
        record.undo.insert(operation_id.to_string(), entry);
        if let Some(review) = record
            .operation
            .as_mut()
            .filter(|review| review.operation.operation_id == operation_id)
        {
            review.can_undo = true;
        }
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn undo_entry(
        &self,
        context: &BackendRequestContext,
        operation_id: &str,
    ) -> Result<(JournalKey, DocumentUndoEntry), BackendError> {
        let (key, record) = self.record_for_operation(context, operation_id)?;
        record
            .undo
            .get(operation_id)
            .cloned()
            .map(|entry| (key, entry))
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Conflict,
                    "La operación no tiene un estado previo para deshacer.",
                    false,
                )
            })
    }

    fn finish_undo(&self, key: &JournalKey, operation_id: &str) -> Result<(), BackendError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        if let Some(record) = records.get_mut(key) {
            record.undo.remove(operation_id);
            if let Some(review) = record.operation.as_mut() {
                if review.operation.operation_id == operation_id {
                    review.can_undo = false;
                }
            }
        }
        Ok(())
    }

    fn trim_records(records: &mut HashMap<JournalKey, JournalRecord>, preserve: &JournalKey) {
        while records.len() > MAX_JOURNAL_RECORDS {
            let eviction_key = records
                .keys()
                .find(|key| *key != preserve)
                .cloned();
            let Some(eviction_key) = eviction_key else { break; };
            records.remove(&eviction_key);
        }
    }

    fn key(context: &BackendRequestContext, idempotency_key: &str) -> JournalKey {
        JournalKey {
            library_id: context.library_id.clone(),
            library_user_id: context.actor.library_user_id.clone(),
            idempotency_key: idempotency_key.to_string(),
        }
    }

    fn record_for_operation(
        &self,
        context: &BackendRequestContext,
        operation_id: &str,
    ) -> Result<(JournalKey, JournalRecord), BackendError> {
        let records = self.records.lock().map_err(|_| internal_error("No se pudo leer el journal backend."))?;
        records
            .iter()
            .find_map(|(key, record)| {
                record.operation.as_ref().and_then(|operation| {
                    (key.library_id == context.library_id
                        && key.library_user_id == context.actor.library_user_id
                        && operation.operation.operation_id == operation_id)
                        .then(|| (key.clone(), record.clone()))
                })
            })
            .ok_or_else(|| BackendError::new(BackendErrorCode::NotFound, "La operación no existe.", false))
    }

    fn store_request(
        &self,
        request: &AgentRequest,
    ) -> Result<(), BackendError> {
        let mut records = self.records.lock().map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(&request.context, &request.idempotency_key);
        let record = records
            .entry(key.clone())
            .or_default();
        record.context = Some(request.context.clone());
        record.request = Some(request.clone());
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    /// What a request left in the journal: the request, each tool call (by
    /// its call key, which starts with the tool name) with its result, and
    /// the final answer when it finished.
    fn turn_record(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Option<(AgentRequest, Vec<(String, ToolResult)>, Option<AgentResponse>)> {
        let records = self.records.lock().ok()?;
        let record = records.get(&Self::key(context, idempotency_key))?;
        let mut tools = record.tool_results.iter().map(|(key, result)| (key.clone(), result.clone())).collect::<Vec<_>>();
        tools.sort_by(|left, right| left.0.cmp(&right.0));
        Some((record.request.clone()?, tools, record.response.clone()))
    }

    fn load_request(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Result<AgentRequest, BackendError> {
        self.records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.request.clone())
            .ok_or_else(|| BackendError::new(
                BackendErrorCode::NotFound,
                "No existe el request necesario para reanudar la operación.",
                true,
            ))
    }

    fn hydrate_record(
        &self,
        app: &AppHandle,
        registry: &LibraryBindingRegistry,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Result<(), BackendError> {
        let key = Self::key(context, idempotency_key);
        if self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .contains_key(&key)
        {
            return Ok(());
        }
        let storage = JournalStorage::open(app, registry, &context.library_id)?;
        let Some(serialized) = crate::database::load_backend_operation_record(
            &storage.connection,
            &context.actor.library_user_id,
            idempotency_key,
        )
        .map_err(|_| storage_error("No se pudo leer el journal backend."))?
        else {
            return Ok(());
        };
        if serialized.len() > MAX_JOURNAL_RECORD_BYTES {
            return Err(BackendError::invalid_input("El journal backend supera el límite de tamaño."));
        }
        let record = serde_json::from_str::<JournalRecord>(&serialized)
            .map_err(|_| storage_error("El journal backend está corrupto."))?;
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        records.insert(key.clone(), record);
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn persist_record(
        &self,
        app: &AppHandle,
        registry: &LibraryBindingRegistry,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Result<(), BackendError> {
        let record = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .cloned();
        let Some(record) = record else { return Ok(()); };
        let serialized = serde_json::to_string(&record.without_binary_content())
            .map_err(|_| internal_error("No se pudo serializar el journal backend."))?;
        if serialized.len() > MAX_JOURNAL_RECORD_BYTES {
            // A long run can outgrow the saved record; its answer is not
            // lost for that. The record stays in memory for this session.
            log::warn!(
                "[notia:journal] registro de {} KB no guardado: supera el límite",
                serialized.len() / 1024
            );
            return Ok(());
        }
        let storage = JournalStorage::open(app, registry, &context.library_id)?;
        crate::database::save_backend_operation_record(
            &storage.connection,
            &context.actor.library_user_id,
            idempotency_key,
            &serialized,
        )
        .map_err(|_| storage_error("No se pudo guardar el journal backend."))?;
        storage.commit(app)
    }
}

/// SQLite connection of the library that owns a journal record. Desktop uses
/// the canonical `.notia` database; Android works on the plugin-managed copy
/// and must sync it back through SAF before a write counts as persisted.
struct JournalStorage {
    connection: rusqlite::Connection,
    #[cfg(target_os = "android")]
    tree_uri: String,
}

impl JournalStorage {
    fn open(
        app: &AppHandle,
        registry: &LibraryBindingRegistry,
        library_id: &str,
    ) -> Result<Self, BackendError> {
        let binding = registry.lookup(library_id)?;
        match binding.root {
            #[cfg(not(target_os = "android"))]
            Some(LibraryBindingRoot::Desktop { canonical_root }) => {
                let _ = app;
                let connection = crate::database::open_existing_library_connection_rw(&canonical_root)
                    .map_err(|_| storage_error("No se pudo abrir la base de datos de la biblioteca."))?;
                crate::database::migrate(&connection)
                    .map_err(|_| storage_error("No se pudo actualizar el journal backend."))?;
                Ok(Self { connection })
            }
            #[cfg(target_os = "android")]
            Some(LibraryBindingRoot::Android { tree_uri }) => {
                let connection =
                    crate::database::open_mobile_library_connection(app, tree_uri.as_str())
                        .map_err(|_| storage_error(
                            "No se pudo abrir la base de datos de la biblioteca. Volvé a autorizar la carpeta.",
                        ))?;
                Ok(Self {
                    connection,
                    tree_uri: tree_uri.as_str().to_string(),
                })
            }
            _ => Err(BackendError::new(
                BackendErrorCode::Unsupported,
                "La biblioteca no tiene un almacenamiento compatible con esta plataforma.",
                true,
            )),
        }
    }

    fn commit(self, app: &AppHandle) -> Result<(), BackendError> {
        #[cfg(target_os = "android")]
        {
            drop(self.connection);
            crate::database::sync_mobile_library_connection(app, &self.tree_uri).map_err(|_| {
                storage_error("No se pudo sincronizar el journal backend. Volvé a autorizar la carpeta.")
            })?;
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (self, app);
        }
        Ok(())
    }
}

impl AgentStateHooks for BackendJournal {
    fn load_response(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Result<Option<AgentResponse>, BackendError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.response.clone()))
    }

    fn store_response(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        response: &AgentResponse,
    ) -> Result<(), BackendError> {
        let mut records = self.records.lock().map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(context, idempotency_key);
        let record = records.entry(key.clone()).or_default();
        record.context = Some(context.clone());
        record.response = Some(response.clone());
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn load_tool_result(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        call_key: &str,
    ) -> Result<Option<ToolResult>, BackendError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.tool_results.get(call_key).cloned()))
    }

    fn store_tool_result(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        call_key: &str,
        result: &ToolResult,
    ) -> Result<(), BackendError> {
        let mut records = self.records.lock().map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(context, idempotency_key);
        let record = records.entry(key.clone()).or_default();
        record.context = Some(context.clone());
        record.tool_results.insert(call_key.to_string(), result.clone());
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn load_continuation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        operation_id: &str,
    ) -> Result<Option<AgentContinuation>, BackendError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.continuation.clone())
            .filter(|continuation| continuation.pending_call.id == operation_id))
    }

    fn store_continuation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        operation_id: &str,
        continuation: &AgentContinuation,
    ) -> Result<(), BackendError> {
        if continuation.pending_call.id != operation_id {
            return Err(BackendError::invalid_input(
                "El checkpoint no corresponde a la operación.",
            ));
        }
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(context, idempotency_key);
        let record = records.entry(key.clone()).or_default();
        record.context = Some(context.clone());
        record.continuation = Some(continuation.clone());
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn clear_continuation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        operation_id: &str,
    ) -> Result<(), BackendError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        if let Some(record) = records.get_mut(&Self::key(context, idempotency_key)) {
            if record
                .continuation
                .as_ref()
                .is_some_and(|continuation| continuation.pending_call.id == operation_id)
            {
                record.continuation = None;
            }
        }
        Ok(())
    }
}

impl OperationStatePort for BackendJournal {
    fn load_operation(
        &self,
        context: &BackendRequestContext,
        operation_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<OperationReview>, BackendError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.operation.clone())
            .filter(|review| review.operation.operation_id == operation_id))
    }

    fn store_operation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        review: &OperationReview,
    ) -> Result<(), BackendError> {
        let mut records = self.records.lock().map_err(|_| internal_error("No se pudo escribir el journal backend."))?;
        let key = Self::key(context, idempotency_key);
        let record = records.entry(key.clone()).or_default();
        record.context = Some(context.clone());
        record.operation = Some(review.clone());
        Self::trim_records(&mut records, &key);
        Ok(())
    }

    fn load_operation_for_request(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
    ) -> Result<Option<OperationReview>, BackendError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| internal_error("No se pudo leer el journal backend."))?
            .get(&Self::key(context, idempotency_key))
            .and_then(|record| record.operation.clone()))
    }
}

impl OperationReviewPort for BackendJournal {
    fn review_operation(
        &self,
        context: &BackendRequestContext,
        operation: &OperationToken,
    ) -> Result<OperationReview, BackendError> {
        self.record_for_operation(context, &operation.operation_id)
            .and_then(|(_, record)| record.operation.ok_or_else(|| {
                BackendError::new(BackendErrorCode::NotFound, "La revisión no existe.", false)
            }))
    }

    fn undo_operation(
        &self,
        _context: &BackendRequestContext,
        _request: &UndoRequest,
    ) -> Result<UndoResult, BackendError> {
        Err(BackendError::new(
            BackendErrorCode::Unsupported,
            "El undo todavía no tiene un adaptador nativo para esta operación.",
            false,
        ))
    }
}

struct TauriRevisionPort {
    app: AppHandle,
}

/// Review/undo port with filesystem access. Review data comes from the
/// journal; undo restores the recorded state only when the document still has
/// the revision the operation produced, so later edits are never overwritten.
struct TauriOperationReviewPort {
    app: AppHandle,
    journal: Arc<BackendJournal>,
}

impl OperationReviewPort for TauriOperationReviewPort {
    fn review_operation(
        &self,
        context: &BackendRequestContext,
        operation: &OperationToken,
    ) -> Result<OperationReview, BackendError> {
        self.journal.review_operation(context, operation)
    }

    fn undo_operation(
        &self,
        context: &BackendRequestContext,
        request: &UndoRequest,
    ) -> Result<UndoResult, BackendError> {
        let operation_id = &request.operation.operation_id;
        let (key, entry) = self.journal.undo_entry(context, operation_id)?;
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let locator = DocumentLocatorDto::new(&context.library_id, &entry.logical_path, None, None)?;
        let reader = crate::library_document_adapter::TauriLibraryDocumentReadAdapter::for_library(
            registry.inner(),
            &context.library_id,
            picker.inner(),
        )
        .map(|reader| reader.with_app(self.app.clone()))?;
        let current = match reader.read_document(&locator) {
            Ok(document) => Some(document),
            Err(error) if error.code == BackendErrorCode::NotFound => None,
            Err(error) => return Err(error),
        };
        if current.as_ref().map(|document| document.revision) != entry.resulting_revision {
            return Err(BackendError::new(
                BackendErrorCode::Conflict,
                "El documento cambió después de la operación; no se deshace para no perder esos cambios.",
                false,
            ));
        }
        let writer = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
            registry.inner(),
            &context.library_id,
            picker.inner(),
        )?;
        match (&entry.previous_content, current) {
            (None, Some(_)) => writer.delete_locator(&locator)?,
            (Some(content), Some(current)) => {
                writer.write_locator_at_revision(&locator, content, current.revision)?
            }
            (Some(content), None) => writer.create_text_locator(&locator, content)?,
            (None, None) => {}
        }
        self.journal.finish_undo(&key, operation_id)?;
        let _ = crate::agent_history::mark_undone(&self.app, &context.library_id, operation_id);
        announce_documents_changed(&self.app, &context.library_id, Some(&json!({ "path": entry.logical_path })));
        Ok(UndoResult {
            operation: request.operation.clone(),
            result: ToolResult {
                call_id: operation_id.clone(),
                ok: true,
                changed: true,
                data: Some(json!({
                    "path": entry.logical_path,
                    "restored": entry.previous_content.is_some(),
                    "deleted": entry.previous_content.is_none(),
                })),
                error: None,
                preview: None,
            },
        })
    }
}

/// The agent wrote notes of a library (`{ libraryId, paths }`): the
/// interface reloads the open ones right away, without waiting for the end
/// of the turn.
pub(crate) const DOCUMENTS_CHANGED_EVENT: &str = "notia://library-documents-changed";

/// The agent moved a file or folder of a library (`{ libraryId, from, to }`,
/// as the explorer and the tabs show them): the open tabs follow it.
pub(crate) const ENTRY_MOVED_EVENT: &str = "notia://library-entry-moved";

fn announce_entry_moved(app: &AppHandle, library_id: &str, data: Option<&Value>) {
    use crate::host::Emitter;
    let path = |field: &str| data.and_then(|data| data.get(field)).and_then(Value::as_str).map(str::to_string);
    let (Some(from), Some(to)) = (path("from"), path("path")) else { return };
    let catalog = app.try_state::<crate::library_catalog::LibraryCatalogState>().is_some();
    let visible = |path: String| if catalog { crate::library_session::visible_path(app, library_id, &path) } else { path };
    let _ = app.emit(ENTRY_MOVED_EVENT, json!({ "libraryId": library_id, "from": visible(from), "to": visible(to) }));
}

fn is_document_write_tool(name: &str) -> bool {
    matches!(
        name,
        "create_library_note"
            | "replace_library_document"
            | "delete_library_document"
            | "apply_markdown_edit"
            | "apply_multi_document_markdown_edit"
    )
}

/// Logical paths a document tool reports: `path`, or `documents[].path`.
fn changed_document_paths(data: Option<&Value>) -> Vec<String> {
    let Some(data) = data else { return Vec::new() };
    let single = data.get("path").and_then(Value::as_str).map(str::to_string);
    let many = data
        .get("documents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|document| document.get("path").and_then(Value::as_str).map(str::to_string));
    single.into_iter().chain(many).collect()
}

fn announce_documents_changed(app: &AppHandle, library_id: &str, data: Option<&Value>) {
    use crate::host::Emitter;
    let paths = changed_document_paths(data);
    if paths.is_empty() {
        return;
    }
    // The tabs hold the path the explorer shows; without the catalog (a
    // headless test), the paths inside the library.
    let catalog = app.try_state::<crate::library_catalog::LibraryCatalogState>().is_some();
    let visible_paths = paths
        .iter()
        .map(|path| if catalog { crate::library_session::visible_path(app, library_id, path) } else { path.clone() })
        .collect::<Vec<_>>();
    let _ = app.emit(DOCUMENTS_CHANGED_EVENT, json!({ "libraryId": library_id, "paths": paths, "visiblePaths": visible_paths }));
}

/// How a cancelled request ended, for the reflection on it.
const CANCELLED_TURN_ENDING: &str = "La persona canceló el pedido antes de que terminara; lo hecho hasta ahí quedó hecho.";

/// Maximum number of skill files loaded into one system prompt.
const MAX_AGENT_SKILLS: usize = 16;
/// Characters of every skill together: the agent files already take most
/// of `MAX_SYSTEM_PROMPT_CHARS`, so the skills past this are left out.
const MAX_AGENT_SKILLS_CHARS: usize = 100_000;
/// Maximum characters read from one agent file (prompt, rules, memory,
/// thoughts, biography, talk, people, skill). The configured models have long
/// contexts, so the memory, the thoughts and the biography may grow large
/// (their own limits sit below this one).
const MAX_AGENT_FILE_CHARS: usize = 250_000;
/// Maximum characters of the composed system prompt (about 150k tokens).
const MAX_SYSTEM_PROMPT_CHARS: usize = 600_000;

/// `.agent` storage of one library through the platform document adapters
/// (canonical desktop root or SAF tree/document resolution on Android).
struct TauriAgentStateRepository {
    app: AppHandle,
}

impl TauriAgentStateRepository {
    /// The whole text of an agent file, frontmatter included, within
    /// `MAX_AGENT_FILE_CHARS`; `None` when it does not exist.
    fn read_raw(&self, library_id: &str, logical_path: &str) -> Result<Option<String>, BackendError> {
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let reader = crate::library_document_adapter::TauriLibraryDocumentReadAdapter::for_library(
            registry.inner(),
            library_id,
            picker.inner(),
        )
        .map(|reader| reader.with_app(self.app.clone()))?;
        let locator = DocumentLocatorDto::new(library_id, logical_path, None, None)?;
        match reader.read_document(&locator) {
            Ok(document) => {
                if notia_backend_core::strip_frontmatter(&document.content).trim().chars().count() > MAX_AGENT_FILE_CHARS {
                    return Err(BackendError::invalid_input(
                        "Un archivo del agente supera el límite permitido.",
                    ));
                }
                Ok(Some(document.content))
            }
            Err(error) if error.code == BackendErrorCode::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The body of an agent file without its frontmatter; `None` when it
    /// does not exist or is empty.
    fn read_optional(&self, library_id: &str, logical_path: &str) -> Result<Option<String>, BackendError> {
        Ok(self.read_raw(library_id, logical_path)?.and_then(|content| {
            let body = notia_backend_core::strip_frontmatter(&content).trim();
            (!body.is_empty()).then(|| body.to_string())
        }))
    }
}

impl notia_backend_core::AgentStateRepository for TauriAgentStateRepository {
    fn load_prompt(&self, library_id: &str, prompt_name: &str) -> Result<Option<String>, BackendError> {
        self.read_optional(library_id, &format!(".agent/promps/{prompt_name}"))
    }

    fn load_memory(&self, library_id: &str, _library_user_id: &str) -> Result<Option<String>, BackendError> {
        // The core only asks for memory after checking the owner and the
        // persistence policy; memory is stored once per library.
        self.read_optional(library_id, ".agent/memory/memory.md")
    }

    fn save_memory(&self, _library_id: &str, _library_user_id: &str, _content: &str) -> Result<(), BackendError> {
        // Memory writes go through the `add_agent_memory` tool, which only
        // the owner gets under a persistent policy.
        Err(unsupported("La memoria del agente solo se modifica con una herramienta confirmada."))
    }

    fn load_rules(&self, library_id: &str) -> Result<Option<String>, BackendError> {
        self.read_optional(library_id, ".agent/memory/rules.md")
    }

    fn load_thoughts(&self, library_id: &str) -> Result<Option<String>, BackendError> {
        // Asked for under the same owner and persistence checks as memory;
        // the agent writes them only with the `add_agent_thought` tool.
        self.read_optional(library_id, notia_backend_core::agent_workspace::THOUGHTS_PATH)
    }

    fn load_biography(&self, library_id: &str) -> Result<Option<String>, BackendError> {
        // Same owner and persistence checks as memory; written with the
        // `add_agent_biography` tool and the reflection after each turn.
        self.read_optional(library_id, notia_backend_core::agent_workspace::BIOGRAPHY_PATH)
    }

    fn load_talk(&self, library_id: &str) -> Result<Option<String>, BackendError> {
        // Same owner and persistence checks as memory; written with the
        // `add_agent_talk` tool and the reflection after each turn.
        self.read_optional(library_id, notia_backend_core::agent_workspace::TALK_PATH)
    }

    fn load_people(&self, library_id: &str) -> Result<Option<String>, BackendError> {
        // Same owner and persistence checks as memory; written with the
        // `add_agent_person` tool, the reflection after each turn and its
        // organization, which also takes people from the memory.
        self.read_optional(library_id, notia_backend_core::agent_workspace::PEOPLE_PATH)
    }

    /// Every Markdown file under `.agent/skills` (also in subfolders), in
    /// path order, with its frontmatter so the prompt shows its `name` and
    /// `description`. A skill that cannot be read or does not fit in
    /// `MAX_AGENT_SKILLS_CHARS` is left out instead of failing the turn.

    fn load_skills(&self, library_id: &str) -> Result<Vec<notia_backend_core::AgentSkill>, BackendError> {
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let reader = crate::library_document_adapter::TauriLibraryDocumentReadAdapter::for_library(
            registry.inner(),
            library_id,
            picker.inner(),
        )
        .map(|reader| reader.with_app(self.app.clone()))?;
        let page = match reader.inventory(&notia_backend_core::InventoryRequest {
            library_id: library_id.to_string(),
            prefix: Some(notia_backend_core::LogicalPathDto::new(".agent/skills")?),
            offset: 0,
            limit: MAX_AGENT_SKILLS,
        }) {
            Ok(page) => page,
            Err(error) if error.code == BackendErrorCode::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut skills = Vec::new();
        let mut total_chars = 0;
        for item in page.items {
            let path = item.locator.logical_path.as_str().to_string();
            if !path.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            let content = match self.read_raw(library_id, &path) {
                Ok(Some(content)) => content,
                Ok(None) => continue,
                Err(error) => {
                    log::warn!("[notia:agent] una habilidad no se pudo leer y quedó afuera: {:?}", error.code);
                    continue;
                }
            };
            let chars = notia_backend_core::strip_frontmatter(&content).trim().chars().count();
            if chars == 0 {
                continue;
            }
            if total_chars + chars > MAX_AGENT_SKILLS_CHARS {
                log::warn!("[notia:agent] una habilidad de {chars} caracteres no entra en el prompt y quedó afuera");
                continue;
            }
            total_chars += chars;
            skills.push(notia_backend_core::AgentSkill { path, content });
        }
        Ok(skills)
    }
}

/// Builds the system prompt once per request from the embedded defaults and
/// the library's `.agent` files through the core prompt policy.
fn compose_request_system_prompt(
    app: &AppHandle,
    request: &AgentRequest,
    visible_tools: &[notia_backend_core::ToolDefinition],
) -> Result<String, BackendError> {
    let repository = TauriAgentStateRepository { app: app.clone() };
    let mut parts = notia_backend_core::load_prompt_parts_with_request(
        &repository,
        &request.context,
        &notia_backend_core::PromptLoadRequest {
            // Without an explicit prompt, the one selected for the library
            // on this device applies (also for Telegram and Meeting).
            prompt_name: request
                .prompt_name
                .clone()
                .map(|name| notia_backend_core::agent_workspace::normalize_prompt_file_name(&name))
                .unwrap_or_else(|| crate::agent_workspace::selected_prompt(app, &request.context.library_id)),
            // The conversational base when the routing call chose it for a Telegram turn.
            default_prompt: notia_backend_core::base_agent_prompt(request.conversational).to_string(),
            default_rules: notia_backend_core::DEFAULT_AGENT_RULES.to_string(),
            synchronize_default_prompt: false,
        },
    )?;
    let format = notia_backend_core::response_format_for_channel(&request.context.channel);
    parts.rules = parts
        .rules
        .map(|rules| notia_backend_core::resolve_rules_for_format(&rules, format))
        .filter(|rules| !rules.is_empty());
    let mut system = notia_backend_core::compose_system_prompt(&parts, &request.context);
    // The local time lets the agent follow rules such as quiet hours and
    // tell which of its thoughts already passed.
    system.push_str(&format!("\n\nFecha y hora local: {}.", crate::local_time::local_now().1));
    if request.autonomous {
        system.push_str("\n\n");
        system.push_str(&notia_backend_core::agent_autonomy::autonomous_guidance());
    }
    if let Some(action) = &request.scheduled_action {
        system.push_str("\n\n");
        system.push_str(&notia_backend_core::ai_actions::prompts::action_guidance(action));
    }
    if let Some(call) = &request.webhook_call {
        system.push_str("\n\n");
        system.push_str(&notia_backend_core::webhooks::webhook_guidance(call));
    }
    if let Some(turn) = &request.ai_task {
        system.push_str("\n\n");
        system.push_str(&notia_backend_core::ai_tasks::prompts::phase_guidance(turn.phase));
    }
    let guidance = notia_backend_core::scope_guidance(
        &request.context,
        visible_tools,
        request.snapshot.as_ref(),
        &utc_today(),
    );
    if !guidance.is_empty() {
        system.push_str("\n\n");
        system.push_str(&guidance);
    }
    let tool_names = visible_tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>();
    if let Some(mail) = crate::mail_tools::guidance(app, &request.context.library_id, &tool_names) {
        system.push_str("\n\n");
        system.push_str(&mail);
    }
    system.push_str(&format!(
        "\n\nContexto backend: scope {:?}, canal {:?}, política {:?}. No afirmes mutaciones sin receipt.",
        request.context.scope,
        request.context.channel,
        request.context.persistence_policy,
    ));
    if system.chars().count() > MAX_SYSTEM_PROMPT_CHARS {
        return Err(BackendError::invalid_input(
            "El prompt compuesto supera el límite permitido.",
        ));
    }
    Ok(system)
}

/// Replaces any client-supplied system message with the backend-owned prompt.
struct AuthoritativePromptProvider<'a, T> {
    inner: &'a T,
    /// Composes the system prompt for the tools of a round: its guidance
    /// depends on them, and a routed run changes them mid-run.
    compose: Box<dyn Fn(&[notia_backend_core::ToolDefinition]) -> Result<String, BackendError> + Send + Sync + 'a>,
    /// The last prompt, by the tool names it was composed for.
    composed: Mutex<Option<(Vec<String>, String)>>,
}

impl<'a, T: AgentProvider> AuthoritativePromptProvider<'a, T> {
    fn system_prompt(&self, tools: &[notia_backend_core::ToolDefinition]) -> Result<String, BackendError> {
        let names = tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>();
        let mut composed = self.composed.lock().map_err(|_| storage_error("No se pudo preparar el prompt del agente."))?;
        if let Some((cached, prompt)) = composed.as_ref() {
            // A round without tools keeps the guidance of the run's tools.
            if *cached == names || names.is_empty() {
                return Ok(prompt.clone());
            }
        }
        let prompt = (self.compose)(tools)?;
        *composed = Some((names, prompt.clone()));
        Ok(prompt)
    }

    fn request(&self, request: &ProviderRequest) -> Result<ProviderRequest, BackendError> {
        let system_prompt = self.system_prompt(&request.tools)?;
        let mut messages = request
            .messages
            .iter()
            .filter(|message| message.role != ProviderMessageRole::System)
            .cloned()
            .collect::<Vec<_>>();
        messages.insert(
            0,
            ProviderMessage {
                role: ProviderMessageRole::System,
                content: system_prompt,
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_name: None,
            },
        );
        Ok(ProviderRequest {
            context: request.context.clone(),
            messages,
            tools: request.tools.clone(),
        })
    }
}

impl<'a, T: AgentProvider> AgentProvider for AuthoritativePromptProvider<'a, T> {
    fn chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
    ) -> Result<ProviderResponse, BackendError> {
        self.inner.chat(&self.request(request)?, control)
    }

    fn stream_chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(ProviderStreamDelta) -> Result<(), BackendError>,
    ) -> Result<ProviderResponse, BackendError> {
        self.inner.stream_chat(&self.request(request)?, control, on_delta)
    }

    fn tool_chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
    ) -> Result<ProviderResponse, BackendError> {
        self.inner.tool_chat(&self.request(request)?, control)
    }
}

impl RevisionPort for TauriRevisionPort {
    fn current_revision(
        &self,
        context: &BackendRequestContext,
        logical_path: &str,
    ) -> Result<Option<u64>, BackendError> {
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let reader = crate::library_document_adapter::TauriLibraryDocumentReadAdapter::for_library(
            registry.inner(),
            &context.library_id,
            picker.inner(),
        )
        .map(|reader| reader.with_app(self.app.clone()))?;
        let locator = DocumentLocatorDto::new(&context.library_id, logical_path, None, None)?;
        match reader.read_document(&locator) {
            Ok(document) => Ok(Some(document.revision)),
            // A note that does not exist yet has revision 0, the one a
            // creation expects: created meanwhile, it no longer matches.
            Err(error) if error.code == BackendErrorCode::NotFound => Ok(Some(MISSING_DOCUMENT_REVISION)),
            Err(error) => Err(error),
        }
    }
}

/// Revision of a document that does not exist, as a creation preview expects.
const MISSING_DOCUMENT_REVISION: u64 = 0;

struct TauriBackendToolExecutor {
    app: AppHandle,
    snapshot: Option<BackendSnapshot>,
    web_search_tracker: Mutex<WebSearchTracker>,
    /// Control of the owning request; blocking provider calls made by tools
    /// honour the same cancellation and deadline as the agent loop.
    control: RequestControl,
    journal: Arc<BackendJournal>,
    idempotency_key: String,
    /// Images of the person's last message, for the tools that keep one
    /// (a recipe's photo).
    request_images: Vec<crate::recipes::MessageImage>,
    /// Image attachments of the conversation (the person's and the images
    /// the agent made) by the reference a note inserts them with
    /// (`![…](reference)`) and an edit names, as base64.
    attachment_images: Vec<(String, String)>,
    /// Events of the request, for the tools that stream what they do (the
    /// terminal's output); `None` outside a run.
    events: Option<TauriBackendEventSink>,
    /// Whether someone follows the run live (a chat or Telegram, not an AI
    /// task), so the terminal asks the light agent for its status.
    live_status: bool,
}

impl TauriBackendToolExecutor {
    fn supported_tool_names() -> &'static [&'static str] {
        &[
            "request_user_clarification",
            "request_user_confirmation",
            "search_web",
            "get_weather",
            "set_agent_execution_plan",
            "set_task_execution_plan",
            "create_agent_plan",
            "update_agent_plan",
            "get_workspace_context",
            "add_agent_memory",
            "add_agent_rule",
            "add_agent_thought",
            "add_agent_biography",
            "add_agent_talk",
            "add_agent_person",
            "add_agent_improvement",
            "generate_image",
            "edit_image",
            "run_terminal_command",
            "browser_navigate",
            "browser_snapshot",
            "browser_click",
            "browser_type",
            "browser_press_key",
            "browser_select_option",
            "browser_screenshot",
            "browser_tabs",
            "browser_wait",
            "browser_close",
            "browser_drag",
            "whatsapp_list_chats",
            "whatsapp_read_messages",
            "whatsapp_send_message",
            "whatsapp_send_file",
            "whatsapp_get_image",
            "list_agent_scripts",
            "run_agent_script",
            "save_agent_script",
            "read_library_documents",
            "search_library_documents",
            "search_library_context",
            "search_library_exact",
            "get_document_metadata",
            "inspect_page_chain",
            "find_document_references",
            "compare_documents",
            "read_active_markdown_document",
            "create_library_note",
            "replace_library_document",
            "delete_library_document",
            "move_library_entry",
            "preview_markdown_edit",
            "apply_markdown_edit",
            "preview_multi_document_markdown_edit",
            "apply_multi_document_markdown_edit",
            "reindex_changed_documents",
            "export_document",
            "search_task_tickets",
            "read_task_tickets",
            "read_all_task_tickets",
            "search_task_context",
            "get_task_manager_options",
            "get_task_board_summary",
            "get_task_board_settings",
            "get_task_board_plan",
            "set_task_time",
            "set_task_people",
            "set_task_project",
            "start_task_timer",
            "pause_task_timer",
            "set_task_block_note",
            "set_task_dependencies",
            "toggle_task_subtask",
            "add_task_board_member",
            "remove_task_board_member",
            "add_task_board_project",
            "remove_task_board_project",
            "update_task_board_calendar",
            "create_task_ticket",
            "replace_task_content",
            "add_task_comment",
            "update_task_comment",
            "delete_task_comment",
            "add_task_subtask",
            "move_task_group",
            "change_task_state",
            "change_task_priority",
            "update_task_fields",
            "bulk_update_tasks",
            "duplicate_task",
            "archive_task",
            "restore_task",
            "create_task_group",
            "update_task_group",
            "reorder_task_groups",
            "delete_task_group",
            "get_finance_dashboard",
            "get_finance_full_snapshot",
            "get_finance_dollar_quotes",
            "get_finance_inflation_indices",
            "get_finance_historical_dollar_quotes",
            "get_finance_record",
            "list_finance_movements",
            "list_finance_records",
            "list_finance_accounts",
            "list_finance_categories",
            "search_finance_categories",
            "list_finance_services",
            "list_finance_service_occurrences",
            "list_finance_service_invoices",
            "list_finance_review_items",
            "list_finance_products",
            "list_finance_merchants",
            "list_finance_installment_plans",
            "list_finance_installments",
            "list_finance_purchases",
            "list_finance_salaries",
            "list_finance_price_history",
            "list_finance_credit_card_statements",
            "list_finance_artifacts",
            "extract_finance_document",
            "save_finance_account",
            "save_finance_category",
            "save_finance_transaction",
            "create_finance_transaction",
            "save_finance_service",
            "save_finance_service_occurrence",
            "save_finance_service_invoice",
            "create_finance_service",
            "create_finance_service_occurrence",
            "create_finance_service_invoice",
            "create_finance_savings_movement",
            "create_finance_savings_exchange",
            "create_finance_category",
            "create_finance_purchase",
            "create_finance_salary",
            "create_finance_credit_card_statement",
            "save_finance_savings_reserve",
            "save_finance_savings_movement",
            "save_finance_savings_exchange",
            "save_finance_purchase",
            "save_finance_salary",
            "save_finance_credit_card_statement",
            "save_finance_installment_plan",
            "link_finance_savings_account",
            "link_finance_records",
            "unlink_finance_records",
            "resolve_finance_review_item",
            "rename_finance_product",
            "rename_finance_merchant",
            "merge_finance_products",
            "merge_finance_merchants",
            "set_finance_service_active",
            "delete_finance_record",
            "reverse_finance_transaction",
            "update_finance_transaction_status",
            "clear_finance_data",
            "get_routine_dashboard",
            "get_routine_day",
            "list_routine_history",
            "get_routine_month_report",
            "save_routine",
            "delete_routine",
            "save_routine_task",
            "set_routine_task_status",
            "delete_routine_task",
            "restore_routine_task",
            "reorder_routine_tasks",
            "set_routine_completions",
            "set_routine_goal",
            "list_recipes",
            "get_recipe",
            "create_recipe",
            "update_recipe",
            "delete_recipe",
            "get_health_summary",
            "list_health_records",
            "save_health_profile",
            "log_weight",
            "save_body_measurement",
            "set_weight_goal",
            "set_health_plan",
            "log_water",
            "log_steps",
            "log_sleep",
            "log_meal",
            "update_meal",
            "delete_health_record",
            "get_gym_summary",
            "get_gym_routine",
            "search_gym_exercises",
            "get_gym_exercise",
            "list_gym_equipment",
            "list_gym_workouts",
            "save_gym_routine",
            "delete_gym_routine",
            "set_gym_equipment",
            "control_gym_session",
            "log_gym_workout",
            "delete_gym_workout",
            "save_gym_exercise",
            "delete_gym_exercise",
            "save_gym_equipment",
            "delete_gym_equipment",
            "list_ai_actions",
            "get_ai_action",
            "create_ai_action",
            "update_ai_action",
            "set_ai_action_enabled",
            "delete_ai_action",
            "run_ai_action_now",
            "retry_ai_action_run",
            "list_ai_tasks",
            "get_ai_task",
            "create_ai_task",
            "cancel_ai_task",
            "resume_ai_task",
            "review_ai_task",
            "delete_ai_task",
            "wait_ai_task",
            "list_agenda",
            "create_agenda_event",
            "delete_agenda_event",
            "add_agenda_note",
            "set_agenda_note_done",
            "delete_agenda_note",
            "get_meeting_overview",
            "update_meeting_notes",
            "add_meeting_notes",
            "set_meeting_ai_context",
            "correct_meeting_transcript",
            "list_gmail_messages",
            "read_gmail_message",
            "list_gmail_labels",
            "trash_gmail_messages",
            "move_gmail_messages",
            "mark_gmail_spam",
            "mark_gmail_read",
            "send_gmail_message",
            "list_calendar_events",
            "create_calendar_event",
        ]
    }

    fn reader<'a>(
        &'a self,
        library_id: &str,
    ) -> Result<crate::library_document_adapter::TauriLibraryDocumentReadAdapter<'a>, BackendError>
    {
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        crate::library_document_adapter::TauriLibraryDocumentReadAdapter::for_library(
            registry.inner(),
            library_id,
            picker.inner(),
        )
        .map(|reader| reader.with_app(self.app.clone()))
    }

    fn paths(arguments: &Value) -> Vec<String> {
        arguments
            .get("paths")
            .or_else(|| arguments.get("logicalPaths"))
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn ids(arguments: &Value) -> Vec<String> {
        arguments
            .get("documentIds")
            .or_else(|| arguments.get("ids"))
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Non-empty trimmed strings of an array argument, bounded to `limit`.
    fn string_list(arguments: &Value, name: &str, limit: usize) -> Vec<String> {
        arguments
            .get(name)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .take(limit)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn text(arguments: &Value, name: &str) -> String {
        arguments
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string()
    }

    fn first_text(arguments: &Value, primary: &str, fallback: &str) -> String {
        let value = Self::text(arguments, primary);
        if value.is_empty() {
            Self::text(arguments, fallback)
        } else {
            value
        }
    }

    /// Logical path given explicitly as `path`, `logicalPath` or
    /// `relativePath` (the name used by the canonical tool schemas).
    fn explicit_path(arguments: &Value) -> Option<String> {
        ["path", "logicalPath", "relativePath"]
            .into_iter()
            .map(|name| Self::text(arguments, name))
            .find(|value| !value.is_empty())
    }

    fn locator(
        context: &BackendRequestContext,
        arguments: &Value,
    ) -> Result<DocumentLocatorDto, BackendError> {
        let path = Self::explicit_path(arguments).ok_or_else(|| {
            BackendError::invalid_input("La mutación documental necesita una ruta lógica.")
        })?;
        DocumentLocatorDto::new(&context.library_id, &path, None, None)
    }

    /// Where a new note goes: the requested path with names Windows and
    /// Android accept (`portable_document_path`), so a title with quotes or
    /// `?` still becomes a note. The preview and the write use the same path.
    fn new_note_locator(
        context: &BackendRequestContext,
        arguments: &Value,
    ) -> Result<DocumentLocatorDto, BackendError> {
        let path = Self::explicit_path(arguments).ok_or_else(|| {
            BackendError::invalid_input("La mutación documental necesita una ruta lógica.")
        })?;
        let path = notia_backend_core::library_tools::portable_document_path(&path);
        DocumentLocatorDto::new(&context.library_id, &path, None, None)
    }

    /// Resolves an existing document by explicit path or by the opaque
    /// `documentId` returned from search results. Ids are resolved through the
    /// library inventory, never interpreted as paths.
    fn existing_document_locator(
        &self,
        context: &BackendRequestContext,
        arguments: &Value,
    ) -> Result<DocumentLocatorDto, BackendError> {
        if Self::explicit_path(arguments).is_some() {
            return Self::locator(context, arguments);
        }
        let document_id = Self::text(arguments, "documentId");
        if document_id.is_empty() {
            return Err(BackendError::invalid_input(
                "La operación documental necesita documentId o una ruta lógica.",
            ));
        }
        self.reader(&context.library_id)?
            .read_documents(&context.library_id, &[document_id])?
            .into_iter()
            .next()
            .map(|document| document.locator)
            .ok_or_else(|| {
                BackendError::new(BackendErrorCode::NotFound, "El documento no existe.", false)
            })
    }

    /// The move a `move_library_entry` call asks for: the source by path
    /// (a file or folder) or by the `documentId` of a search.
    fn entry_move(
        &self,
        context: &BackendRequestContext,
        arguments: &Value,
    ) -> Result<notia_backend_core::library_tools::EntryMove, BackendError> {
        let source = match Self::explicit_path(arguments) {
            Some(path) => path,
            None => self.existing_document_locator(context, arguments)?.logical_path.as_str().to_string(),
        };
        notia_backend_core::library_tools::plan_entry_move(&source, &Self::text(arguments, "targetFolder"))
    }

    /// What the confirmation of a move shows. A missing source or a taken
    /// destination goes back to the model before asking the person.
    fn entry_move_summary(&self, context: &BackendRequestContext, call: &ToolCall) -> Result<String, BackendError> {
        let entry_move = self.entry_move(context, &call.arguments)?;
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let exists = |path: &str| {
            crate::filesystem::commands::library_entry_exists(registry.inner(), picker.inner(), &context.library_id, path)
        };
        if !exists(entry_move.source.as_str())? {
            return Err(BackendError::new(
                BackendErrorCode::NotFound,
                format!("No existe «{}» en la biblioteca.", entry_move.source.as_str()),
                false,
            ));
        }
        if exists(entry_move.destination.as_str())? {
            return Err(BackendError::new(
                BackendErrorCode::Conflict,
                format!("Ya existe «{}»: elegí otra carpeta o renombrá el archivo primero.", entry_move.destination.as_str()),
                false,
            ));
        }
        let folder = entry_move.target_folder.as_ref().map_or("la raíz de la biblioteca", |folder| folder.as_str());
        Ok(format!("Mover «{}» a «{}».", entry_move.source.as_str(), folder))
    }

    /// Refuses an agent write to a Task Manager index: the store rebuilds it
    /// on every change, so boards and groups change only through the Task
    /// Manager tools. The editor still saves the file as any other note.
    fn ensure_agent_writable(locator: &DocumentLocatorDto) -> Result<(), BackendError> {
        if crate::task_manager_store::is_task_manager_index(locator.logical_path.as_str()) {
            return Err(BackendError::new(
                BackendErrorCode::Forbidden,
                "Los índices de Task Manager no se editan como documentos: los grupos, su orden, nombre y color se cambian con las herramientas de Task Manager.",
                false,
            ));
        }
        if crate::recipes::is_recipe_path(locator.logical_path.as_str()) {
            return Err(BackendError::new(
                BackendErrorCode::Forbidden,
                "Las recetas no se editan como documentos: se cargan y se cambian con create_recipe, update_recipe y delete_recipe, que las revisan con IA.",
                false,
            ));
        }
        Ok(())
    }

    fn markdown_edit_request(
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<MarkdownEditRequest, BackendError> {
        let locator = Self::locator(context, &call.arguments)?;
        Self::ensure_agent_writable(&locator)?;
        let operation_value = call
            .arguments
            .get("operation")
            .or_else(|| call.arguments.get("edit"))
            .cloned()
            .ok_or_else(|| BackendError::invalid_input("La edición Markdown necesita una operación."))?;
        let operation = serde_json::from_value(operation_value).map_err(invalid_markdown_operation)?;
        Ok(MarkdownEditRequest {
            operation_id: call.id.clone(),
            locator,
            expected_revision: revision_argument(&call.arguments),
            operation,
        })
    }

    /// The preview of a `preview_markdown_edit` or `apply_markdown_edit`
    /// call, computed from the current document. Both take the same path and
    /// operation, so the apply never needs the preview back from the model.
    fn markdown_preview(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<MarkdownEditPreview, BackendError> {
        let request = Self::markdown_edit_request(context, call)?;
        let document = self.reader(&context.library_id)?.read_document(&request.locator)?;
        preview_markdown_edit(&document.content, &request)
    }

    fn preview_markdown_tool(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Value, BackendError> {
        Ok(compact_markdown_preview(&self.markdown_preview(context, call)?))
    }

    /// Builds a multi-document request from the canonical tool schema
    /// (`edits[{path, operation}]`). Returns `None` for the legacy shape with
    /// explicit locators.
    fn path_based_multi_edit(
        context: &BackendRequestContext,
        arguments: &Value,
    ) -> Result<Option<MultiDocumentMarkdownEditRequest>, BackendError> {
        let Some(edits) = arguments.get("edits").and_then(Value::as_array) else {
            return Ok(None);
        };
        if !edits.iter().all(|edit| edit.get("path").is_some()) {
            return Ok(None);
        }
        let edits = edits
            .iter()
            .map(|edit| {
                let operation = edit
                    .get("operation")
                    .cloned()
                    .ok_or_else(|| BackendError::invalid_input("Cada edición necesita una operación."))?;
                Ok(notia_backend_core::MarkdownDocumentEdit {
                    locator: Self::locator(context, edit)?,
                    operation: serde_json::from_value(operation).map_err(invalid_markdown_operation)?,
                })
            })
            .collect::<Result<Vec<_>, BackendError>>()?;
        Ok(Some(MultiDocumentMarkdownEditRequest {
            operation_id: String::new(),
            edits,
        }))
    }

    /// The preview of a `preview_multi_document_markdown_edit` or
    /// `apply_multi_document_markdown_edit` call, computed from the current
    /// documents. An edit with `expectedRevision` fails when its document
    /// changed since.
    fn multi_markdown_preview(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<MultiDocumentMarkdownPreview, BackendError> {
        let mut request = match Self::path_based_multi_edit(context, &call.arguments)? {
            Some(request) => request,
            None => {
                let value = call
                    .arguments
                    .get("request")
                    .cloned()
                    .unwrap_or_else(|| call.arguments.clone());
                serde_json::from_value::<MultiDocumentMarkdownEditRequest>(value).map_err(|_| {
                    BackendError::invalid_input("La edición multi-documento no es válida.")
                })?
            }
        };
        request.operation_id = call.id.clone();
        for edit in &mut request.edits {
            if edit.locator.library_id != context.library_id {
                return Err(BackendError::new(
                    BackendErrorCode::Forbidden,
                    "La edición multi-documento cruza la biblioteca activa.",
                    false,
                ));
            }
            let path = edit.locator.logical_path.as_str().to_string();
            edit.locator = DocumentLocatorDto::new(&context.library_id, &path, None, None)?;
            Self::ensure_agent_writable(&edit.locator)?;
        }
        let reader = self.reader(&context.library_id)?;
        let preview = preview_multi_document_markdown_edit(&reader, &request)?;
        for edit in call.arguments.get("edits").and_then(Value::as_array).into_iter().flatten() {
            let Some(expected) = revision_argument(edit) else {
                continue;
            };
            let path = Self::locator(context, edit)?.logical_path.as_str().to_string();
            let changed = preview
                .previews
                .iter()
                .any(|document| document.locator.logical_path.as_str() == path && document.expected_revision != expected);
            if changed {
                return Err(BackendError::new(
                    BackendErrorCode::Conflict,
                    format!("El documento {path} cambió desde el preview; releelo y volvé a pedir el cambio."),
                    true,
                ));
            }
        }
        Ok(preview)
    }

    /// Writes into a Markdown apply call the revisions its confirmation
    /// showed, per edit for a multi-document apply.
    fn with_confirmed_revisions(arguments: &mut Value, preview: &MutationPreview) {
        let revision = |path: &str| {
            preview
                .documents
                .iter()
                .find(|document| document.path == path.trim())
                .map(|document| document.expected_revision)
        };
        if let Some(edits) = arguments.get_mut("edits").and_then(Value::as_array_mut) {
            for edit in edits {
                let revision = Self::explicit_path(edit).and_then(|path| revision(&path));
                if let (Some(revision), Some(edit)) = (revision, edit.as_object_mut()) {
                    edit.insert("expectedRevision".to_string(), json!(revision));
                }
            }
        } else if let (Some(document), Some(arguments)) = (preview.documents.first(), arguments.as_object_mut()) {
            arguments.insert("expectedRevision".to_string(), json!(document.expected_revision));
        }
    }

    fn preview_multi_markdown_tool(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Value, BackendError> {
        let preview = self.multi_markdown_preview(context, call)?;
        Ok(json!({
            "documents": preview.previews.iter().map(compact_markdown_preview).collect::<Vec<_>>(),
        }))
    }

    /// `content` with the images of the person's message it inserts
    /// (`![…](reference)`) embedded as JPEG, scaled down as a recipe's photo.
    /// The previews keep the short reference; only the written file holds
    /// the image.
    fn with_message_images(&self, content: String) -> Result<String, BackendError> {
        use base64::Engine as _;
        use notia_backend_core::chat_attachments::{embed_images, inserts_image};
        let mut images = Vec::new();
        for (reference, base64) in &self.attachment_images {
            if !inserts_image(&content, reference) {
                continue;
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(base64.trim())
                .map_err(|_| BackendError::invalid_input(format!("La imagen «{reference}» del mensaje no se pudo leer.")))?;
            let photo = crate::recipes::prepare_photo(&bytes).map_err(|error| BackendError::invalid_input(error.message))?;
            images.push((reference.clone(), photo.data_uri()));
        }
        Ok(if images.is_empty() { content } else { embed_images(&content, &images) })
    }

    fn apply_markdown_tool(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Value, BackendError> {
        let preview = self.markdown_preview(context, call)?;
        let selected_hunk_ids = call
            .arguments
            .get("selectedHunkIds")
            .or_else(|| call.arguments.get("hunkIds"))
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let Some(content) = materialize_markdown_preview(&preview, &selected_hunk_ids)? else {
            return Ok(json!({ "changed": false, "operationId": preview.operation_id }));
        };
        let content = self.with_message_images(content)?;
        let content = crate::folder_contexts::enforce(&self.app, &context.library_id, preview.locator.logical_path.as_str(), content);
        let reader = self.reader(&context.library_id)?;
        let current = reader.read_document(&preview.locator)?;
        if current.revision != preview.expected_revision {
            return Err(BackendError::new(
                BackendErrorCode::Conflict,
                "El documento Markdown cambió desde el preview.",
                true,
            ));
        }
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let writer = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
            registry.inner(),
            &context.library_id,
            picker.inner(),
        )?;
        writer.write_locator_at_revision(&preview.locator, &content, preview.expected_revision)?;
        Ok(json!({
            "changed": true,
            "operationId": preview.operation_id,
            "path": preview.locator.logical_path.as_str(),
            "revision": crate::backend::compute_document_revision(&content),
        }))
    }

    /// Applies a multi-document preview all-or-nothing: every document is
    /// materialized and its revision verified before the first write, and
    /// documents already written are restored if a later write fails.
    fn apply_multi_markdown_tool(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Value, BackendError> {
        let preview = self.multi_markdown_preview(context, call)?;
        let selected = call
            .arguments
            .get("selectedHunkIds")
            .or_else(|| call.arguments.get("hunkIds"))
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let known = preview
            .previews
            .iter()
            .flat_map(|document| document.hunks.iter().map(|hunk| hunk.id.as_str()))
            .collect::<std::collections::HashSet<_>>();
        if selected.iter().any(|id| !known.contains(id.as_str())) {
            return Err(BackendError::invalid_input(
                "Se seleccionó un hunk que no pertenece al preview.",
            ));
        }
        let reader = self.reader(&context.library_id)?;
        let mut planned = Vec::new();
        for document in &preview.previews {
            let path = document.locator.logical_path.as_str().to_string();
            let ids = if selected.is_empty() {
                Vec::new()
            } else {
                let ids = document
                    .hunks
                    .iter()
                    .filter(|hunk| selected.contains(&hunk.id))
                    .map(|hunk| hunk.id.clone())
                    .collect::<Vec<_>>();
                if ids.is_empty() {
                    continue;
                }
                ids
            };
            let Some(content) = materialize_markdown_preview(document, &ids)? else {
                continue;
            };
            let current = reader.read_document(&document.locator)?;
            if current.revision != document.expected_revision {
                return Err(BackendError::new(
                    BackendErrorCode::Conflict,
                    format!("El documento {path} cambió desde el preview; no se aplicó ningún cambio."),
                    true,
                ));
            }
            let content = self.with_message_images(content)?;
            let content = crate::folder_contexts::enforce(&self.app, &context.library_id, document.locator.logical_path.as_str(), content);
            planned.push((document.locator.clone(), current.content, content));
        }
        let registry = self.app.state::<LibraryBindingRegistry>();
        let picker = self.app.state::<AndroidDirectoryPickerState>();
        let writer = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
            registry.inner(),
            &context.library_id,
            picker.inner(),
        )?;
        let mut written: Vec<&(DocumentLocatorDto, String, String)> = Vec::new();
        for entry in &planned {
            let (locator, original, content) = entry;
            let revision = crate::backend::compute_document_revision(original);
            if let Err(error) = writer.write_locator_at_revision(locator, content, revision) {
                let not_restored = written
                    .iter()
                    .rev()
                    .filter(|(locator, original, content)| {
                        writer
                            .write_locator_at_revision(
                                locator,
                                original,
                                crate::backend::compute_document_revision(content),
                            )
                            .is_err()
                    })
                    .map(|(locator, _, _)| locator.logical_path.as_str().to_string())
                    .collect::<Vec<_>>();
                let message = if not_restored.is_empty() {
                    format!("{} No se aplicó ningún cambio.", error.message)
                } else {
                    format!(
                        "{} Quedaron modificados sin poder revertirse: {}.",
                        error.message,
                        not_restored.join(", ")
                    )
                };
                return Err(BackendError::new(error.code, message, error.retryable));
            }
            written.push(entry);
        }
        // Each document says the frontmatter it now has where it changed,
        // so a long chain edit shows the last link it set.
        let last = planned.last().map(|(locator, _, _)| locator.logical_path.as_str().to_string());
        Ok(json!({
            "changed": !planned.is_empty(),
            "operationId": preview.operation_id,
            "summary": match &last {
                Some(last) => format!("Se aplicaron los {} documentos de la tanda; el último fue {last}.", planned.len()),
                None => "No había nada que cambiar.".to_string(),
            },
            "documents": planned
                .iter()
                .map(|(locator, original, content)| json!({
                    "path": locator.logical_path.as_str(),
                    "revision": crate::backend::compute_document_revision(content),
                    "frontmatter": notia_backend_core::markdown_editing::frontmatter_changes(original, content),
                }))
                .collect::<Vec<_>>(),
        }))
    }

    /// Desktop root path or Android tree URI of the request's library.
    fn library_location(
        &self,
        context: &BackendRequestContext,
    ) -> Result<(String, Option<String>), BackendError> {
        let registry = self.app.state::<LibraryBindingRegistry>();
        let binding = registry.lookup(&context.library_id)?;
        Ok(match binding.root {
            Some(LibraryBindingRoot::Desktop { canonical_root }) => {
                (canonical_root.to_string_lossy().to_string(), None)
            }
            Some(LibraryBindingRoot::Android { tree_uri }) => {
                (String::new(), Some(tree_uri.as_str().to_string()))
            }
            None => {
                return Err(BackendError::new(
                    BackendErrorCode::NotFound,
                    "La biblioteca no está disponible.",
                    true,
                ))
            }
        })
    }

    fn routine_context(
        &self,
        context: &BackendRequestContext,
    ) -> Result<crate::routine::RoutineContext, BackendError> {
        let (library_path, android_directory_uri) = self.library_location(context)?;
        let source = match context.channel {
            notia_backend_core::BackendChannel::Telegram => "telegram",
            _ => "app",
        };
        Ok(crate::routine::RoutineContext {
            library_path,
            android_directory_uri,
            actor_library_user_id: context.actor.library_user_id.clone(),
            source: source.to_string(),
        })
    }

    /// Salud of the acting library user.
    fn gym_context(context: &BackendRequestContext) -> crate::gym::GymContext {
        crate::gym::GymContext {
            library_id: context.library_id.clone(),
            actor_library_user_id: context.actor.library_user_id.clone(),
        }
    }

    /// The boards the Task Manager tools may touch: in a published turn,
    /// only the ones the running publication shares; otherwise all of them.
    fn task_board_scope(&self, context: &BackendRequestContext) -> Result<Vec<String>, BackendError> {
        if !matches!(context.persistence_policy, PersistencePolicy::PublishedNoMemory) {
            return Ok(Vec::new());
        }
        crate::task_manager_publication::published_board_ids(&self.app, &context.library_id).ok_or_else(|| {
            BackendError::new(BackendErrorCode::Forbidden, "La publicación de Task Manager ya no está activa.", false)
        })
    }

    fn health_context(context: &BackendRequestContext) -> crate::health::HealthContext {
        crate::health::HealthContext {
            library_id: context.library_id.clone(),
            actor_library_user_id: context.actor.library_user_id.clone(),
        }
    }

    /// The library database of the AI actions, which are the Owner's only.
    fn ai_actions_database(&self, context: &BackendRequestContext) -> Result<crate::library_users::LibraryDatabaseContext, BackendError> {
        if !context.actor.is_library_owner() {
            return Err(BackendError::new(BackendErrorCode::Forbidden, "Las acciones de IA son solo del Owner de la biblioteca.", false));
        }
        let (library_path, android_directory_uri) = self.library_location(context)?;
        Ok(crate::library_users::LibraryDatabaseContext { library_path, android_directory_uri })
    }

    fn agenda_context(&self, context: &BackendRequestContext) -> Result<crate::agenda::AgendaContext, BackendError> {
        let (library_path, android_directory_uri) = self.library_location(context)?;
        Ok(crate::agenda::AgendaContext {
            library_path,
            android_directory_uri,
            actor_library_user_id: context.actor.library_user_id.clone(),
        })
    }

    fn agenda_error(error: crate::agenda::AgendaCommandError) -> BackendError {
        use crate::agenda::AgendaErrorCode;
        let code = match error.code {
            AgendaErrorCode::Validation => BackendErrorCode::InvalidInput,
            AgendaErrorCode::NotFound => BackendErrorCode::NotFound,
            AgendaErrorCode::Storage => BackendErrorCode::Storage,
        };
        BackendError::new(code, error.message, code == BackendErrorCode::Storage)
    }

    fn routine_error(error: crate::routine::RoutineCommandError) -> BackendError {
        use crate::routine::RoutineErrorCode;
        let code = match error.code {
            RoutineErrorCode::Validation => BackendErrorCode::InvalidInput,
            RoutineErrorCode::NotFound => BackendErrorCode::NotFound,
            RoutineErrorCode::Conflict => BackendErrorCode::Conflict,
            RoutineErrorCode::Storage => BackendErrorCode::Storage,
        };
        BackendError::new(code, error.message, code == BackendErrorCode::Storage)
    }

    fn finance_context(
        &self,
        context: &BackendRequestContext,
    ) -> Result<crate::finance::FinanceContext, BackendError> {
        let (library_path, android_directory_uri) = self.library_location(context)?;
        let source = match context.channel {
            notia_backend_core::BackendChannel::Telegram => "telegram",
            notia_backend_core::BackendChannel::Published => "public-url",
            _ => "app",
        };
        Ok(crate::finance::FinanceContext {
            library_path,
            android_directory_uri,
            actor_library_user_id: context.actor.library_user_id.clone(),
            source: source.to_string(),
        })
    }

    fn page_limit(arguments: &Value, name: &str, default: usize) -> usize {
        arguments
            .get(name)
            .and_then(Value::as_u64)
            .unwrap_or(default as u64)
            .min(50) as usize
    }

    fn optional_text(arguments: &Value, name: &str) -> Option<String> {
        arguments
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    /// The payload of a `save_finance_*` tool that takes a whole record, or
    /// why the record is not valid. The reason names the field, so the model
    /// can fix its call.
    fn finance_save_payload<T: serde::de::DeserializeOwned>(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<T, BackendError> {
        let (key, invalid) = match call.name.as_str() {
            "save_finance_account" => ("account", "La cuenta financiera no es válida"),
            "save_finance_category" => ("category", "La categoría financiera no es válida"),
            "save_finance_transaction" => ("transaction", "El movimiento financiero no es válido"),
            "save_finance_service" => ("service", "El servicio financiero no es válido"),
            "save_finance_service_occurrence" => ("occurrence", "La ocurrencia financiera no es válida"),
            "save_finance_service_invoice" => ("invoice", "La factura financiera no es válida"),
            "save_finance_savings_movement" => ("movement", "El movimiento de ahorro no es válido"),
            _ => return Err(BackendError::invalid_input("La herramienta financiera no guarda un registro.")),
        };
        let finance_context = self.finance_context(context)?;
        let mut record = Self::finance_record(&call.arguments);
        if call.name == "save_finance_transaction" {
            record = self.with_stored_transaction(&finance_context, record);
        }
        let payload = json!({ "context": finance_context, key: record });
        serde_json::from_value::<T>(payload.clone()).map_err(|error| {
            let missing = Self::missing_fields::<T>(&payload, key);
            if missing.is_empty() {
                BackendError::invalid_input(format!("{invalid}: {error}."))
            } else {
                BackendError::invalid_input(format!("{invalid}: faltan los campos {}.", missing.join(", ")))
            }
        })
    }

    /// A movement edit over the movement already saved with that id: the
    /// fields sent replace the stored ones, so changing only the description
    /// does not need the whole record.
    fn with_stored_transaction(&self, finance_context: &crate::finance::FinanceContext, record: Value) -> Value {
        let Some(id) = record.get("id").and_then(Value::as_str).map(str::to_string) else {
            return record;
        };
        let stored = crate::finance::finance_get_transaction(
            self.app.clone(),
            crate::finance::GetFinanceRecordPayload { context: finance_context.clone(), id },
        );
        let (Ok(stored), Some(changes)) = (stored, record.as_object()) else {
            return record;
        };
        let mut merged = serde_json::to_value(stored).unwrap_or_default();
        if let Some(object) = merged.as_object_mut() {
            for (field, value) in changes.iter().filter(|(_, value)| !value.is_null()) {
                object.insert(field.clone(), value.clone());
            }
        }
        merged
    }

    /// Every field a record lacks: serde names one per try, so each is filled
    /// with a placeholder and the record is tried again.
    fn missing_fields<T: serde::de::DeserializeOwned>(payload: &Value, key: &str) -> Vec<String> {
        const MAX_TRIES: usize = 40;
        let mut candidate = payload.clone();
        let mut missing = Vec::new();
        for _ in 0..MAX_TRIES {
            let Err(error) = serde_json::from_value::<T>(candidate.clone()) else { break };
            let message = error.to_string();
            let Some(field) = message.strip_prefix("missing field `").and_then(|rest| rest.split('`').next()).map(str::to_string) else {
                break;
            };
            if missing.contains(&field) {
                break;
            }
            match candidate.get_mut(key).and_then(Value::as_object_mut) {
                Some(record) => record.insert(field.clone(), Value::String(String::new())),
                None => break,
            };
            missing.push(field);
        }
        missing
    }

    /// Checks the record of a `save_finance_*` call without saving it, so an
    /// incomplete record returns to the model before the person confirms
    /// something that could only fail.
    fn validate_finance_save(&self, context: &BackendRequestContext, call: &ToolCall) -> Result<(), BackendError> {
        match call.name.as_str() {
            "save_finance_account" => self.finance_save_payload::<crate::finance::SaveAccountPayload>(context, call).map(drop),
            "save_finance_category" => self.finance_save_payload::<crate::finance::SaveCategoryPayload>(context, call).map(drop),
            "save_finance_transaction" => self.finance_save_payload::<crate::finance::SaveTransactionPayload>(context, call).map(drop),
            "save_finance_service" => self.finance_save_payload::<crate::finance::SaveFinanceServicePayload>(context, call).map(drop),
            "save_finance_service_occurrence" => self.finance_save_payload::<crate::finance::SaveFinanceServiceOccurrencePayload>(context, call).map(drop),
            "save_finance_service_invoice" => self.finance_save_payload::<crate::finance::SaveFinanceServiceInvoicePayload>(context, call).map(drop),
            "save_finance_savings_movement" => self.finance_save_payload::<crate::finance::SaveSavingsMovementPayload>(context, call).map(drop),
            _ => Ok(()),
        }
    }

    fn finance_record(arguments: &Value) -> Value {
        let mut record = arguments
            .get("record")
            .cloned()
            .unwrap_or_else(|| arguments.clone());
        if let Some(object) = record.as_object_mut() {
            let missing_id = object
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if missing_id {
                object.insert("id".to_string(), Value::String(uuid::Uuid::new_v4().to_string()));
            }
        }
        record
    }

    fn finance_period(
        arguments: &Value,
        name: &str,
        required: bool,
    ) -> Result<Option<String>, BackendError> {
        let Some(value) = arguments.get(name) else {
            if required {
                return Err(BackendError::invalid_input(format!(
                    "La consulta financiera necesita {name}."
                )));
            }
            return Ok(None);
        };
        let period = value.as_str().map(str::trim).unwrap_or_default();
        if !crate::finance::valid_service_period(period) {
            return Err(BackendError::invalid_input(format!(
                "{name} debe tener formato YYYY-MM."
            )));
        }
        Ok(Some(period.to_string()))
    }

    fn finance_date(arguments: &Value, name: &str) -> Result<Option<String>, BackendError> {
        let value = Self::optional_text(arguments, name);
        if let Some(date) = value.as_deref() {
            let valid = date.len() == 10
                && date.is_ascii()
                && date.as_bytes().get(4) == Some(&b'-')
                && date.as_bytes().get(7) == Some(&b'-')
                && date[0..4].parse::<u16>().is_ok()
                && (1..=12).contains(&date[5..7].parse::<u8>().unwrap_or_default())
                && (1..=31).contains(&date[8..10].parse::<u8>().unwrap_or_default());
            if !valid {
                return Err(BackendError::invalid_input(format!("{name} debe tener formato YYYY-MM-DD.")));
            }
        }
        Ok(value)
    }

    fn finance_id(arguments: &Value, name: &str) -> Result<String, BackendError> {
        let value = arguments
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if value.is_empty() {
            return Err(BackendError::invalid_input(format!(
                "La consulta financiera necesita {name}."
            )));
        }
        if value.chars().count() > 128 || value.chars().any(char::is_control) {
            return Err(BackendError::invalid_input(format!(
                "{name} no es válido."
            )));
        }
        Ok(value.to_string())
    }

    /// Accounts and categories of the current month used to resolve agent
    /// references by name before any finance write.
    /// `get_finance_record` of an entity other than a movement: found by its
    /// id in the entity's list; a purchase also brings its lines.
    fn finance_record_by_id(&self, context: &BackendRequestContext, entity: &str, id: &str) -> Result<Value, BackendError> {
        let finance_context = self.finance_context(context)?;
        let all_periods = || crate::finance_records::ListPeriodPayload {
            context: finance_context.clone(),
            from: None,
            to: None,
            merchant_id: None,
            product_id: None,
        };
        let app = || self.app.clone();
        let records = match entity {
            "purchases" => serde_json::to_value(crate::finance_records::finance_list_purchases(app(), all_periods()).map_err(Self::finance_error)?),
            "salaries" => serde_json::to_value(crate::finance_records::finance_list_salaries(app(), all_periods()).map_err(Self::finance_error)?),
            "credit_card_statements" => serde_json::to_value(
                crate::finance_records::finance_list_credit_card_statements(app(), all_periods()).map_err(Self::finance_error)?,
            ),
            "installment_plans" => serde_json::to_value(
                crate::finance_records::finance_list_installment_plans(app(), finance_context.clone()).map_err(Self::finance_error)?,
            ),
            "services" => serde_json::to_value(crate::finance::finance_list_services(app(), finance_context.clone()).map_err(Self::finance_error)?),
            "service_occurrences" => serde_json::to_value(
                crate::finance::finance_list_all_service_occurrences(app(), finance_context.clone()).map_err(Self::finance_error)?,
            ),
            "service_invoices" => serde_json::to_value(
                crate::finance::finance_list_service_invoices(app(), finance_context.clone(), None).map_err(Self::finance_error)?,
            ),
            "accounts" => serde_json::to_value(crate::finance::finance_list_accounts(app(), finance_context.clone()).map_err(Self::finance_error)?),
            "categories" => serde_json::to_value(self.finance_dashboard(&finance_context)?.categories),
            "artifacts" => serde_json::to_value(
                crate::services::finance_extraction::list_finance_artifacts(app(), finance_context.clone()).map_err(Self::finance_error)?,
            ),
            _ => return Err(BackendError::invalid_input(format!("get_finance_record no lee registros de tipo {entity}."))),
        }
        .map_err(|_| invalid_result())?;
        let mut record = records
            .as_array()
            .and_then(|records| records.iter().find(|record| record.get("id").and_then(Value::as_str) == Some(id)))
            .cloned()
            .ok_or_else(|| BackendError::new(BackendErrorCode::NotFound, format!("No hay un registro de {entity} con id {id}."), false))?;
        if entity == "purchases" {
            record["items"] = Value::Array(crate::finance_records::purchase_items(&self.app, &finance_context, id).map_err(Self::finance_error)?);
        }
        Ok(json!({ "ok": true, "entity": entity, "record": record }))
    }

    fn finance_dashboard(
        &self,
        finance_context: &crate::finance::FinanceContext,
    ) -> Result<crate::finance::FinanceDashboard, BackendError> {
        crate::finance::finance_get_dashboard(
            self.app.clone(),
            finance_context.clone(),
            utc_today()[..7].to_string(),
        )
        .map_err(Self::finance_error)
    }

    /// Validates a finance creation before confirmation. Invalid input and
    /// duplicates return `InvalidInput`, which the agent loop hands back to
    /// the model instead of asking the user to confirm a doomed write.
    fn finance_create_preview(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Option<(String, Value)>, BackendError> {
        use finance_agent_inputs::CreateOrExisting;
        let name = call.name.as_str();
        if !matches!(
            name,
            "create_finance_category"
                | "create_finance_service"
                | "create_finance_transaction"
                | "create_finance_purchase"
                | "create_finance_salary"
                | "create_finance_credit_card_statement"
        ) {
            return Ok(None);
        }
        let rejected = |error: finance_agent_inputs::AgentInputError| {
            BackendError::invalid_input(format!(
                "{}: {} Campos: {}.",
                error.error,
                error.instruction,
                error.invalid_fields.join(", ")
            ))
        };
        let duplicate = |kind: &str, id: &str, entity_name: &str| {
            BackendError::invalid_input(format!(
                "Ya existe {kind} '{entity_name}' (id {id}); usá ese ID en lugar de crearla de nuevo."
            ))
        };
        let finance_context = self.finance_context(context)?;
        let dashboard = self.finance_dashboard(&finance_context)?;
        let account_name = |id: &str| {
            dashboard
                .accounts
                .iter()
                .find(|account| account.id == id)
                .map(|account| account.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let source_reference = Self::agent_source_reference(context, call);
        let (summary, record) = match name {
            "create_finance_category" => {
                match finance_agent_inputs::build_category(&call.arguments, &dashboard.categories)
                    .map_err(rejected)?
                {
                    CreateOrExisting::Existing(category) => {
                        return Err(duplicate("la categoría", &category.id, &category.name))
                    }
                    CreateOrExisting::Create(category) => (
                        format!(
                            "Crear la categoría {} para {}.",
                            category.name,
                            if category.kind == "expense" { "gastos" } else { "ingresos" }
                        ),
                        serde_json::to_value(category),
                    ),
                }
            }
            "create_finance_service" => {
                let services =
                    crate::finance::finance_list_services(self.app.clone(), finance_context.clone())
                        .map_err(Self::finance_error)?;
                match finance_agent_inputs::build_service(
                    &call.arguments,
                    &dashboard.categories,
                    &services,
                )
                .map_err(rejected)?
                {
                    CreateOrExisting::Existing(service) => {
                        return Err(duplicate("el servicio", &service.id, &service.name))
                    }
                    CreateOrExisting::Create(service) => (
                        format!(
                            "Crear el servicio mensual {} por {} {}.",
                            service.name, service.expected_amount, service.currency
                        ),
                        serde_json::to_value(service),
                    ),
                }
            }
            "create_finance_transaction" if call.arguments.get("splits").is_some_and(|splits| !splits.is_null()) => {
                let parts = Self::split_calls(call).map_err(rejected)?.unwrap_or_default();
                let mut drafts = Vec::new();
                for part in &parts {
                    drafts.push(self.transaction_draft(context, part, &finance_context, &dashboard)?.map_err(rejected)?.transaction);
                }
                let lines = drafts
                    .iter()
                    .map(|transaction| format!("{} {} · {}", transaction.amount, transaction.currency, transaction.description))
                    .collect::<Vec<_>>()
                    .join("; ");
                let account = drafts.first().map(|transaction| account_name(&transaction.account_id)).unwrap_or_default();
                (format!("Registrar {} movimientos, uno por categoría, en {account}: {lines}.", drafts.len()), serde_json::to_value(&drafts))
            }
            "create_finance_transaction" => {
                let draft = self.transaction_draft(context, call, &finance_context, &dashboard)?.map_err(rejected)?;
                if let Some(existing) = self.existing_finance_transaction(&finance_context, &draft)? {
                    return Err(BackendError::invalid_input(format!(
                        "Ese movimiento ya fue registrado (id {}); no lo registres de nuevo.",
                        existing.id
                    )));
                }
                let transaction = &draft.transaction;
                let kind = match transaction.transaction_type.as_str() {
                    "income" => "ingreso",
                    "expense" => "gasto",
                    "transfer" => "transferencia",
                    _ => "ajuste",
                };
                let service = draft
                    .service
                    .as_ref()
                    .map(|service| format!(" (pago de {})", service.name))
                    .unwrap_or_default();
                (
                    format!(
                        "Registrar {kind} de {} {} en {}: {}{service}.",
                        transaction.amount,
                        transaction.currency,
                        account_name(&transaction.account_id),
                        transaction.description
                    ),
                    serde_json::to_value(transaction),
                )
            }
            "create_finance_purchase" => {
                let services =
                    crate::finance::finance_list_services(self.app.clone(), finance_context.clone())
                        .map_err(Self::finance_error)?;
                let purchase = finance_agent_inputs::build_purchase(
                    &call.arguments,
                    &dashboard.accounts,
                    &dashboard.categories,
                    &services,
                    &source_reference,
                    &utc_today(),
                )
                .map_err(rejected)?;
                // A ticket that does not add up goes back before asking.
                let validation = crate::finance_records::validate_purchase(&purchase).map_err(BackendError::invalid_input)?;
                if !validation.valid {
                    return Err(BackendError::invalid_input(crate::finance_records::purchase_mismatch(&purchase, &validation)));
                }
                (
                    format!(
                        "Guardar ticket de {}: {} producto(s), total {} {}, en {}.",
                        purchase.merchant_name,
                        purchase.items.len(),
                        purchase.total_amount,
                        purchase.currency,
                        account_name(&purchase.account_id)
                    ),
                    serde_json::to_value(purchase),
                )
            }
            "create_finance_salary" => {
                let salary = finance_agent_inputs::build_salary(
                    &call.arguments,
                    &dashboard.accounts,
                    &source_reference,
                )
                .map_err(rejected)?;
                (
                    format!(
                        "Guardar recibo de sueldo de {}, período {}, neto {} {}, en {}.",
                        salary.employer,
                        salary.period,
                        salary.net_amount,
                        salary.currency,
                        account_name(&salary.account_id)
                    ),
                    serde_json::to_value(salary),
                )
            }
            _ => {
                let statement = finance_agent_inputs::build_credit_card_statement(
                    &call.arguments,
                    &dashboard.accounts,
                    &source_reference,
                )
                .map_err(rejected)?;
                (
                    format!(
                        "Guardar resumen de {}, período {}, total {} {}, en {}.",
                        statement.issuer,
                        statement.period,
                        statement.total_due,
                        statement.currency,
                        account_name(&statement.account_id)
                    ),
                    serde_json::to_value(statement),
                )
            }
        };
        Ok(Some((summary, record.map_err(|_| invalid_result())?)))
    }

    /// Document and pre-mutation content of an undoable document tool.
    fn undo_target(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Option<(DocumentLocatorDto, Option<String>)>, BackendError> {
        let locator = match call.name.as_str() {
            "create_library_note" => return Ok(Some((Self::new_note_locator(context, &call.arguments)?, None))),
            "replace_library_document" | "delete_library_document" => {
                self.existing_document_locator(context, &call.arguments)?
            }
            "apply_markdown_edit" => match Self::explicit_path(&call.arguments) {
                Some(_) => Self::locator(context, &call.arguments)?,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let previous = self.reader(&context.library_id)?.read_document(&locator)?;
        Ok(Some((locator, Some(previous.content))))
    }

    /// Records the undo entry after a successful document mutation. Large
    /// documents are skipped: the operation stays non-undoable.
    fn record_undo(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
        locator: &DocumentLocatorDto,
        previous_content: Option<String>,
    ) -> Result<(), BackendError> {
        if previous_content
            .as_ref()
            .is_some_and(|content| content.len() > MAX_UNDO_CONTENT_BYTES)
        {
            return Ok(());
        }
        let resulting = match self.reader(&context.library_id)?.read_document(locator) {
            Ok(document) => Some(document),
            Err(error) if error.code == BackendErrorCode::NotFound => None,
            Err(error) => return Err(error),
        };
        let logical_path = locator.logical_path.as_str().to_string();
        // The history is what the interface lists and diffs; a failure to
        // write it must not undo or hide an applied change.
        let _ = crate::agent_history::record(
            &self.app,
            context,
            &call.id,
            &logical_path,
            Self::history_summary(&call.name, &logical_path),
            previous_content.clone(),
            resulting.as_ref().map(|document| document.content.clone()),
        );
        self.journal.store_undo(
            context,
            &self.idempotency_key,
            &call.id,
            DocumentUndoEntry {
                logical_path,
                previous_content,
                resulting_revision: resulting.map(|document| document.revision),
            },
        )
    }

    fn history_summary(tool: &str, logical_path: &str) -> String {
        let action = match tool {
            "create_library_note" => "Creación de",
            "delete_library_document" => "Eliminación de",
            "export_document" => "Exportación a",
            _ => "Edición de",
        };
        format!("{action} {logical_path}")
    }

    /// Records a transaction requested by the agent, once per evidence
    /// reference, and marks the paid service month.
    /// The calls of a movement split into categories, one per part, each
    /// with its own id so each part gets its own source reference.
    fn split_calls(call: &ToolCall) -> Result<Option<Vec<ToolCall>>, finance_agent_inputs::AgentInputError> {
        Ok(finance_agent_inputs::split_parts(&call.arguments)?.map(|parts| {
            parts
                .into_iter()
                .enumerate()
                .map(|(index, arguments)| ToolCall { id: format!("{}-{}", call.id, index + 1), arguments, ..call.clone() })
                .collect()
        }))
    }

    fn create_finance_transaction(&self, context: &BackendRequestContext, call: &ToolCall) -> Result<Value, BackendError> {
        match Self::split_calls(call) {
            Err(error) => return Ok(error.to_value()),
            Ok(Some(parts)) => {
                let saved = parts.iter().map(|part| self.create_finance_transaction(context, part)).collect::<Result<Vec<_>, _>>()?;
                return Ok(json!({
                    "ok": saved.iter().all(|part| part["ok"] == true),
                    "changed": saved.iter().any(|part| part["changed"] == true),
                    "parts": saved,
                }));
            }
            Ok(None) => {}
        }
        let finance_context = self.finance_context(context)?;
        let dashboard = self.finance_dashboard(&finance_context)?;
        let draft = match self.transaction_draft(context, call, &finance_context, &dashboard)? {
            Ok(draft) => draft,
            Err(error) => return Ok(error.to_value()),
        };
        if let Some(existing) = self.existing_finance_transaction(&finance_context, &draft)? {
            return Ok(json!({ "ok": true, "changed": false, "duplicate": true, "transaction": existing }));
        }
        let (saved, outcome) = match crate::finance::finance_save_transaction_linked(
            self.app.clone(),
            crate::finance::SaveTransactionPayload {
                context: finance_context.clone(),
                transaction: draft.transaction.clone(),
            },
        )
        .map_err(Self::finance_error)
        {
            Ok(saved) => saved,
            // A storage error can arrive after the commit (for example while
            // syncing the Android copy): report success only if the record
            // is really there.
            Err(error) if error.code == BackendErrorCode::Storage => {
                match self.existing_finance_transaction(&finance_context, &draft) {
                    Ok(Some(saved)) => (saved, Default::default()),
                    _ => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        let crate::finance_matching::LinkOutcome { links, review_items } = outcome;
        let Some(service) = draft.service else {
            return Ok(json!({ "ok": true, "changed": true, "transaction": saved, "links": links, "reviewItems": review_items }));
        };
        // The payment marks the service's month as paid; a failure is
        // reported so the user can fix the occurrence, never hidden.
        let occurrence = crate::finance::finance_save_service_occurrence(
            self.app.clone(),
            crate::finance::SaveFinanceServiceOccurrencePayload {
                context: finance_context,
                occurrence: crate::finance::FinanceServiceOccurrence {
                    id: uuid::Uuid::new_v4().to_string(),
                    service_id: service.id.clone(),
                    period: saved.effective_date[..7].to_string(),
                    expected_amount: service.expected_amount.clone(),
                    paid_amount: Some(saved.amount.clone()),
                    effective_date: Some(saved.effective_date.clone()),
                    status: "current".to_string(),
                    transaction_id: Some(saved.id.clone()),
                    artifact_id: None,
                    source_reference: saved.source_reference.clone(),
                    raw_source: saved.raw_source.clone(),
                    actor_library_user_id: saved.actor_library_user_id.clone(),
                    source: saved.source.clone(),
                    created_at: None,
                    updated_at: None,
                    difference: None,
                },
                reason: None,
            },
        );
        Ok(match occurrence {
            Ok(occurrence) => json!({ "ok": true, "changed": true, "transaction": saved, "occurrence": occurrence, "links": links, "reviewItems": review_items }),
            Err(error) => json!({
                "ok": false,
                "changed": true,
                "error": "finance-service-occurrence-save-failed",
                "message": "El gasto se guardó, pero no pudo asociarse a la ocurrencia del servicio. Revisá la ocurrencia antes de reintentar.",
                "transaction": saved,
                "occurrenceError": error.message,
            }),
        })
    }

    /// Transaction requested by the agent with its references resolved and
    /// the evidence reference that makes a retried call idempotent.
    fn transaction_draft(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
        finance_context: &crate::finance::FinanceContext,
        dashboard: &crate::finance::FinanceDashboard,
    ) -> Result<Result<finance_agent_inputs::TransactionDraft, finance_agent_inputs::AgentInputError>, BackendError> {
        let services = crate::finance::finance_list_services(self.app.clone(), finance_context.clone())
            .map_err(Self::finance_error)?;
        Ok(finance_agent_inputs::build_transaction(
            &call.arguments,
            &dashboard.accounts,
            &dashboard.categories,
            &services,
            &utc_today(),
            context.channel == notia_backend_core::BackendChannel::Telegram,
        )
        .map(|mut draft| {
            if draft.transaction.source_reference.is_none() {
                draft.transaction.source_reference = Some(Self::agent_source_reference(context, call));
            }
            draft.transaction.source = finance_context.source.clone();
            draft.transaction.actor_library_user_id = Some(context.actor.library_user_id.clone());
            draft
        }))
    }

    fn existing_finance_transaction(
        &self,
        finance_context: &crate::finance::FinanceContext,
        draft: &finance_agent_inputs::TransactionDraft,
    ) -> Result<Option<crate::finance::FinanceTransaction>, BackendError> {
        let Some(reference) = draft.transaction.source_reference.as_deref() else {
            return Ok(None);
        };
        let transactions = crate::finance::finance_list_all_transactions(self.app.clone(), finance_context.clone())
            .map_err(Self::finance_error)?;
        Ok(finance_agent_inputs::existing_transaction(&transactions, reference).cloned())
    }

    /// Stable evidence reference for a record created from chat without an
    /// attachment; retries of the same tool call reuse it.
    fn agent_source_reference(context: &BackendRequestContext, call: &ToolCall) -> String {
        format!("agent:{}:{}", context.request_id, call.id)
    }

    fn finance_error(error: crate::finance::FinanceCommandError) -> BackendError {
        let (code, retryable) = match error.code {
            "notFound" => (BackendErrorCode::NotFound, false),
            "conflict" => (BackendErrorCode::Conflict, false),
            "validation" => (BackendErrorCode::InvalidInput, false),
            _ => (BackendErrorCode::Storage, true),
        };
        BackendError::new(code, error.message, retryable)
    }

    fn append_agent_file(
        &self,
        context: &BackendRequestContext,
        kind: crate::agent_workspace::AgentItem,
        value: &str,
    ) -> Result<Value, BackendError> {
        Self::ensure_agent_memory_writable(context)?;
        let changed = crate::agent_workspace::append_agent_item(&self.app, &context.library_id, kind, value)?;
        // A parallel call without tools and without memory context
        // organizes every agent file together; the turn does not wait for it.
        if changed {
            crate::agent_knowledge::schedule_organization(&self.app, &context.library_id);
        }
        Ok(json!({ "changed": changed }))
    }

    /// The agent files of `.agent/memory` are written only for the owner
    /// under a policy that keeps memory.
    fn ensure_agent_memory_writable(context: &BackendRequestContext) -> Result<(), BackendError> {
        if !context.persistence_policy.allows_memory() {
            return Err(BackendError::new(
                BackendErrorCode::Forbidden,
                "La política de esta superficie no permite persistir memoria del agente.",
                false,
            ));
        }
        if !context.actor.is_library_owner() {
            return Err(BackendError::new(
                BackendErrorCode::Forbidden,
                "Solo el propietario puede modificar la memoria del agente.",
                false,
            ));
        }
        Ok(())
    }

    /// `generate_image` / `edit_image`: draws with the local model and keeps
    /// the image for the reply of the request (see `image_generation`). An
    /// edit starts from an image of the conversation or one this request
    /// made.
    fn image_tool(&self, context: &BackendRequestContext, call: &ToolCall) -> Result<Value, BackendError> {
        use notia_backend_core::image_generation as images;
        let seed = crate::image_generation::random_seed();
        let (job, reference) = if call.name == images::EDIT_IMAGE_TOOL {
            let job = images::edit_job(&call.arguments, seed)?;
            let name = job.reference.clone().unwrap_or_default();
            let base64 = crate::image_generation::find_reference(&context.request_id, &name, &self.attachment_images).ok_or_else(|| {
                BackendError::invalid_input(format!(
                    "No hay una imagen «{name}» en la conversación: usá la referencia que figura en «Imagen adjunta» o la que devolvió generate_image."
                ))
            })?;
            (job, Some(base64))
        } else {
            (images::generate_job(&call.arguments, seed)?, None)
        };
        // A local chat model shares the GPU: Ollama reloads it for the next reply.
        if let Some(url) = local_ollama_url(&self.app, &context.library_id) {
            crate::image_generation::release_ollama_gpu(&url);
        }
        crate::image_generation::generate(&self.app, &self.control, &context.request_id, job, reference)
    }

    fn document_mutation_preview(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Option<(DocumentLocatorDto, String, String, u64)>, BackendError> {
        if !matches!(
            call.name.as_str(),
            "create_library_note" | "replace_library_document" | "delete_library_document"
        ) {
            return Ok(None);
        }
        let locator = if call.name == "create_library_note" {
            Self::new_note_locator(context, &call.arguments)?
        } else {
            self.existing_document_locator(context, &call.arguments)?
        };
        Self::ensure_agent_writable(&locator)?;
        let current = self.reader(&context.library_id)?.read_document(&locator);
        match call.name.as_str() {
            "create_library_note" => {
                match current {
                    Ok(existing) => {
                        return Err(BackendError::new(
                            BackendErrorCode::Conflict,
                            format!("El documento ya existe: {}", existing.locator.logical_path.as_str()),
                            true,
                        ));
                    }
                    Err(error) if error.code == BackendErrorCode::NotFound => {}
                    Err(error) => return Err(error),
                }
                Ok(Some((
                    locator,
                    String::new(),
                    Self::text(&call.arguments, "content"),
                    MISSING_DOCUMENT_REVISION,
                )))
            }
            "replace_library_document" => {
                let document = current?;
                let (next, _) = notia_backend_core::library_tools::replacement_content(&document.content, &Self::text(&call.arguments, "content"));
                Ok(Some((locator, document.content, next, document.revision)))
            }
            "delete_library_document" => {
                let document = current?;
                Ok(Some((locator, document.content, String::new(), document.revision)))
            }
            _ => Ok(None),
        }
    }
}

impl ToolExecutor for TauriBackendToolExecutor {
    fn execute_confirmed(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
        decision: &notia_backend_core::ConfirmationDecision,
        preview: Option<&MutationPreview>,
    ) -> Result<ToolResult, BackendError> {
        if call.name == "request_user_confirmation" {
            return Ok(ToolResult {
                call_id: call.id.clone(),
                ok: true,
                changed: false,
                data: Some(json!({ "accepted": decision.accepted })),
                error: None,
                preview: None,
            });
        }
        let Some(preview) = preview else {
            return self.execute(context, call);
        };
        if matches!(
            call.name.as_str(),
            "replace_library_document" | "delete_library_document"
        ) {
            let mut call = call.clone();
            if let Some(document) = preview.documents.first() {
                if let Some(arguments) = call.arguments.as_object_mut() {
                    arguments.insert(
                        "expectedRevision".to_string(),
                        Value::String(document.expected_revision.to_string()),
                    );
                }
            }
            let _ = decision;
            return self.execute(context, &call);
        }
        if call.name == "create_ai_task" {
            // «Confirmar todos» on the creation lets the task apply its
            // changes without asking; the model never decides it.
            let mut call = call.clone();
            if let Some(arguments) = call.arguments.as_object_mut() {
                arguments.insert("approveAll".to_string(), Value::Bool(decision.approve_all));
            }
            return self.execute(context, &call);
        }
        if matches!(call.name.as_str(), "apply_markdown_edit" | "apply_multi_document_markdown_edit") {
            // The apply computes the preview again: the revisions the person
            // saw make it fail if a document changed while they decided.
            let mut call = call.clone();
            Self::with_confirmed_revisions(&mut call.arguments, preview);
            if !decision.hunk_ids.is_empty() {
                if let Some(arguments) = call.arguments.as_object_mut() {
                    arguments.insert(
                        "selectedHunkIds".to_string(),
                        serde_json::to_value(&decision.hunk_ids).map_err(|_| invalid_result())?,
                    );
                }
            }
            return self.execute(context, &call);
        }
        self.execute(context, call)
    }

    fn preview(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<Option<MutationPreview>, BackendError> {
        if call.name == "apply_markdown_edit" {
            let preview = self.markdown_preview(context, call)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: format!("Aplicar edición Markdown en {}", preview.locator.logical_path.as_str()),
                documents: vec![PreviewDocument {
                    path: preview.locator.logical_path.as_str().to_string(),
                    expected_revision: preview.expected_revision,
                    current_revision: preview.current_revision,
                }],
                hunks: preview
                    .hunks
                    .iter()
                    .map(|hunk| PreviewHunk {
                        id: hunk.id.clone(),
                        document_path: preview.locator.logical_path.as_str().to_string(),
                        start_line: hunk.anchor.start_line,
                        end_line: hunk.anchor.end_line,
                        old_text: hunk.old_text.clone(),
                        new_text: hunk.new_text.clone(),
                    })
                    .collect(),
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::ApplySelected,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if call.name == "apply_multi_document_markdown_edit" {
            let preview = self.multi_markdown_preview(context, call)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: format!("Aplicar edición Markdown en {} documentos", preview.previews.len()),
                documents: preview
                    .previews
                    .iter()
                    .map(|document| PreviewDocument {
                        path: document.locator.logical_path.as_str().to_string(),
                        expected_revision: document.expected_revision,
                        current_revision: document.current_revision,
                    })
                    .collect(),
                hunks: preview
                    .previews
                    .iter()
                    .flat_map(|document| {
                        let path = document.locator.logical_path.as_str().to_string();
                        document.hunks.iter().map(move |hunk| PreviewHunk {
                            id: hunk.id.clone(),
                            document_path: path.clone(),
                            start_line: hunk.anchor.start_line,
                            end_line: hunk.anchor.end_line,
                            old_text: hunk.old_text.clone(),
                            new_text: hunk.new_text.clone(),
                        })
                    })
                    .collect(),
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::ApplySelected,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if notia_backend_core::mail_tools::is_mail_write_tool(&call.name) {
            // Reading the messages it touches lets the person check them; an
            // unknown id or label goes back to the model.
            let (summary, detail) = crate::mail_tools::preview(&self.app, context, call)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("gmail:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if notia_backend_core::terminal::is_terminal_tool(&call.name) {
            // The person sees the whole command and where it runs.
            let (summary, detail) = crate::terminal::preview(&self.app, context, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: "terminal".to_string(),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if notia_backend_core::browser::is_browser_interaction_tool(&call.name) {
            // The element as the page shows it, what goes into it and where.
            let (summary, detail) = crate::browser::preview(&self.app, &call.name, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: "browser".to_string(),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if call.name == notia_backend_core::agent_scripts::SAVE_TOOL {
            // Where the script goes and its whole code.
            let (summary, detail) = crate::agent_scripts::preview(&self.app, context, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: notia_backend_core::agent_scripts::SCRIPTS_DIRECTORY.to_string(),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if notia_backend_core::meeting_tools::is_meeting_write_tool(&call.name) {
            // Each correction with how many times it changes the transcript;
            // without this the chat got a confirmation with nothing to show.
            let detail = crate::meeting::preview_agent_tool(&self.app, &call.name, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: detail.lines().next().unwrap_or("Cambiar la reunión").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("meeting:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if notia_backend_core::whatsapp::is_whatsapp_send_tool(&call.name) {
            // The chat and exactly what reaches it.
            let (summary, detail) = crate::whatsapp::preview(&self.app, context, &call.name, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: "whatsapp".to_string(),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: detail,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::agenda_tools::is_agenda_write_tool(&call.name) {
            // Validation runs in a rolled-back transaction; any rejection
            // other than storage (an overlap, a bad time) goes back to the model.
            let summary = crate::agenda_tools::preview_tool(&self.app, &self.agenda_context(context)?, &call.name, &call.arguments)
                .map_err(|error| match error.code {
                    crate::agenda::AgendaErrorCode::Storage => Self::agenda_error(error),
                    _ => BackendError::invalid_input(error.message),
                })?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.clone(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("agenda:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::recipes::is_recipe_write_tool(&call.name) {
            // A new recipe is reviewed by the AI here: the confirmation shows
            // what it completed, and a repeated dish goes back to the model.
            let summary = crate::recipes::preview_tool(&self.app, &context.library_id, &call.id, &call.name, &call.arguments, &self.request_images)
                .map_err(crate::recipes::tool_error)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Cambiar Recetas").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("recipes:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::health::is_health_write_tool(&call.name) {
            // A meal without values and an AI plan are estimated here: the
            // confirmation shows them, and a rejection goes back to the model.
            let summary = crate::health::preview_tool(&self.app, &Self::health_context(context), &call.id, &call.name, &call.arguments, &self.request_images)
                .map_err(crate::health::tool_error)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Cambiar Salud").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("health:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::gym::is_gym_write_tool(&call.name) {
            // Resolved and tried without saving: a rejection goes back to the
            // model before asking the person.
            let summary = crate::gym::preview_tool(&self.app, &Self::gym_context(context), &call.name, &call.arguments).map_err(crate::gym::tool_error)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Cambiar Gimnasio").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("gym:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::ai_task_runner::is_ai_task_write_tool(&call.name) {
            // Checked as its execution will do it; a rejection goes back to
            // the model before asking the Owner.
            let summary = crate::ai_task_runner::preview_tool(&self.app, context, &call.name, &call.arguments)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Cambiar Tareas IA").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("ai-tasks:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::ai_actions::is_ai_action_write_tool(&call.name) {
            // The call is read and checked as its execution will do it; a
            // rejection goes back to the model before asking the Owner.
            let database = self.ai_actions_database(context)?;
            let summary = crate::ai_actions::preview_tool(&self.app, &context.library_id, &database, &call.name, &call.arguments)
                .map_err(crate::ai_actions::tool_error)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Cambiar Acciones IA").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("ai-actions:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if crate::routine_tools::is_routine_write_tool(&call.name) {
            // Resolution and validation run in a rolled-back transaction; any
            // rejection other than storage goes back to the model.
            let summary = crate::routine_tools::preview_tool(
                &self.app,
                &self.routine_context(context)?,
                &call.name,
                &call.arguments,
            )
            .map_err(|error| match error.code {
                crate::routine::RoutineErrorCode::Storage => Self::routine_error(error),
                _ => BackendError::invalid_input(error.message),
            })?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.lines().next().unwrap_or("Actualizar Rutina").to_string(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("routine:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if let Some((summary, record)) = self.finance_create_preview(context, call)? {
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary,
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("finance:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: serde_json::to_string_pretty(&record).map_err(|_| invalid_result())?,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        self.validate_finance_save(context, call)?;
        if call.name.starts_with("save_finance_")
            || call.name.starts_with("create_finance_")
            || matches!(
                call.name.as_str(),
                "set_finance_service_active"
                    | "link_finance_savings_account"
                    | "delete_finance_record"
                    | "reverse_finance_transaction"
                    | "update_finance_transaction_status"
                    | "clear_finance_data"
                    | "extract_finance_document"
                    | "link_finance_records"
                    | "unlink_finance_records"
                    | "resolve_finance_review_item"
                    | "rename_finance_product"
                    | "rename_finance_merchant"
                    | "merge_finance_products"
                    | "merge_finance_merchants"
            )
        {
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: format!("Guardar registro financiero: {}", call.name),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("finance:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: serde_json::to_string(&call.arguments)
                        .map_err(|_| invalid_result())?,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        let board_tool = notia_backend_core::task_board_tool_input::is_task_board_action_tool(&call.name);
        if call.name == "export_document" || board_tool || notia_backend_core::is_task_mutation_tool(&call.name) {
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: if call.name == "export_document" {
                    format!(
                        "Exportar {} a {}.",
                        Self::text(&call.arguments, "path"),
                        Self::text(&call.arguments, "format").to_uppercase()
                    )
                } else if board_tool {
                    let task_state = self.app.state::<crate::task_manager_commands::TaskManagerBackendState>();
                    let registry = self.app.state::<LibraryBindingRegistry>();
                    crate::task_manager_commands::board_tool_summary(
                        &self.app,
                        task_state.inner(),
                        registry.inner(),
                        &context.library_id,
                        &context.actor.library_user_id,
                        self.task_board_scope(context)?,
                        &call.name,
                        &call.arguments,
                    )?
                } else {
                    let task_state = self.app.state::<crate::task_manager_commands::TaskManagerBackendState>();
                    let registry = self.app.state::<LibraryBindingRegistry>();
                    let snapshot = crate::task_manager_commands::backend_tool_snapshot(
                        &self.app,
                        task_state.inner(),
                        registry.inner(),
                        &context.library_id,
                        &context.actor.library_user_id,
                    )?;
                    let arguments = notia_backend_core::task_manager_tool_input::expand_section_edit(&call.name, &call.arguments, &snapshot.tickets)?;
                    let summary = task_mutation_summary(
                        &notia_backend_core::task_mutation_from_tool(
                            &call.name,
                            &arguments,
                            &snapshot.groups,
                            &mut || "preview".to_string(),
                            0,
                        )?,
                        &snapshot.groups,
                        &snapshot.comments,
                    );
                    notia_backend_core::task_manager_tool_input::section_edit_summary(&call.name, &call.arguments, &snapshot.tickets).unwrap_or(summary)
                },
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: if call.name == "export_document" {
                        format!("export:{}", context.library_id)
                    } else {
                        format!("task-manager:{}", context.library_id)
                    },
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: serde_json::to_string(&call.arguments)
                        .map_err(|_| invalid_result())?,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        if call.name == "move_library_entry" {
            let summary = self.entry_move_summary(context, call)?;
            return Ok(Some(MutationPreview {
                operation_id: call.id.clone(),
                summary: summary.clone(),
                documents: Vec::new(),
                hunks: vec![PreviewHunk {
                    id: call.id.clone(),
                    document_path: format!("library:{}", context.library_id),
                    start_line: 1,
                    end_line: 1,
                    old_text: String::new(),
                    new_text: summary,
                }],
                allowed_actions: vec![
                    MutationPreviewAction::ApplyAll,
                    MutationPreviewAction::Reject,
                    MutationPreviewAction::Cancel,
                ],
            }));
        }
        let Some((locator, old_content, new_content, revision)) =
            self.document_mutation_preview(context, call)?
        else {
            return Ok(None);
        };
        Ok(Some(MutationPreview {
            operation_id: call.id.clone(),
            summary: format!("Mutación documental: {}", call.name),
            documents: vec![PreviewDocument {
                path: locator.logical_path.as_str().to_string(),
                expected_revision: revision,
                current_revision: revision,
            }],
            hunks: vec![PreviewHunk {
                id: call.id.clone(),
                document_path: locator.logical_path.as_str().to_string(),
                start_line: 1,
                end_line: old_content.lines().count().max(1) as u32,
                old_text: old_content,
                new_text: new_content,
            }],
            allowed_actions: vec![
                MutationPreviewAction::ApplyAll,
                MutationPreviewAction::Reject,
                MutationPreviewAction::Cancel,
            ],
        }))
    }

    fn execute(
        &self,
        context: &BackendRequestContext,
        call: &ToolCall,
    ) -> Result<ToolResult, BackendError> {
        let reader = self.reader(&context.library_id)?;
        let undo_target = self.undo_target(context, call)?;
        let data = match call.name.as_str() {
            "search_web" => {
                let request = notia_backend_core::sanitize_web_search_request(
                    &Self::text(&call.arguments, "query"),
                    call.arguments.get("maxResults").and_then(Value::as_u64).map(|value| value as u32),
                    call.arguments.get("freshness").and_then(Value::as_str),
                    &call
                        .arguments
                        .get("domains")
                        .and_then(Value::as_array)
                        .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
                        .unwrap_or_default(),
                )?;
                match self
                    .web_search_tracker
                    .lock()
                    .map_err(|_| internal_error("No se pudo proteger el límite de búsqueda web."))?
                    .reserve(&request)
                {
                    WebSearchReservation::Duplicate => {
                        return Err(BackendError::new(
                            BackendErrorCode::Conflict,
                            "La búsqueda web ya fue realizada en esta solicitud.",
                            false,
                        ));
                    }
                    WebSearchReservation::LimitReached => {
                        return Err(BackendError::new(
                            BackendErrorCode::Conflict,
                            "La solicitud alcanzó el límite de búsquedas web únicas.",
                            false,
                        ));
                    }
                    WebSearchReservation::New => {}
                }
                let settings = provider_settings_for_library(
                    &self.app,
                    &self.app.state::<BackendRuntimeState>(),
                    &context.library_id,
                )?;
                let response = crate::backend_ollama::search_web(
                    &self.app,
                    &crate::services::ai_service::AiHttpSettings {
                        ollama_url: settings.ollama_url,
                        api_key: settings.api_key,
                    },
                    &request.query,
                    request.max_results,
                    &self.control,
                )
                .map_err(|error| match error.code {
                    BackendErrorCode::Cancelled | BackendErrorCode::Timeout | BackendErrorCode::InvalidInput => error,
                    _ => BackendError::new(BackendErrorCode::ProviderUnavailable, "El proveedor de búsqueda no está disponible.", true),
                })?;
                let results = response
                    .results
                    .into_iter()
                    .enumerate()
                    .map(|(index, result)| {
                        let source_name = result.source_name;
                        let url = result.url;
                        json!({
                            "rank": index + 1,
                            "title": result.title,
                            "url": url.clone(),
                            "snippet": result.snippet,
                            "sourceName": source_name.clone(),
                            "publishedAt": result.published_at,
                            "verification": "unverified",
                            "verificationScore": 0,
                            "citation": format!("[{}] {} — {}", index + 1, source_name, url),
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "ok": true,
                    "changed": false,
                    "searchedQuery": request.query,
                    "consistency": if results.len() < 2 { "insufficient" } else { "mixed" },
                    "results": results,
                })
            }
            // Telegram and scheduled runs have no open view: that is the
            // answer, not a failure.
            "get_workspace_context" => match self.snapshot.as_ref() {
                Some(snapshot) => serde_json::to_value(snapshot).map_err(|_| invalid_result())?,
                None => json!({
                    "available": false,
                    "reason": "Este pedido no llega desde una vista abierta de Notia (por ejemplo, Telegram o una acción programada): no hay documento activo, selección ni pestañas. Buscá y leé los documentos con las herramientas de la biblioteca.",
                }),
            },
            "read_library_documents" => {
                let ids = Self::ids(&call.arguments);
                let documents = if ids.is_empty() {
                    let paths = Self::paths(&call.arguments);
                    paths
                        .iter()
                        .map(|path| {
                            let locator = DocumentLocatorDto::new(
                                &context.library_id,
                                path,
                                None,
                                None,
                            )?;
                            reader.read_document(&locator)
                        })
                        .collect::<Result<Vec<DocumentContent>, BackendError>>()?
                } else {
                    reader.read_documents(&context.library_id, &ids)?
                };
                // A long document is read by parts, without embedded photos
                // or the files of a saved chat (see `document_parts`).
                let offset = call.arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let limit = call.arguments.get("limit").and_then(Value::as_u64).map(|limit| limit as usize);
                json!({ "documents": notia_backend_core::library_tools::document_parts(documents, offset, limit) })
            }
            "search_library_documents" => {
                let page = reader.search_documents(&DocumentSearchRequest {
                    library_id: context.library_id.clone(),
                    query: Self::text(&call.arguments, "query"),
                    titles: Self::string_list(&call.arguments, "titles", 20),
                    tags: Self::string_list(&call.arguments, "tags", 12),
                    kind: match Self::text(&call.arguments, "type").as_str() {
                        "markdown" => Some(notia_backend_core::DocumentKind::Markdown),
                        "text" => Some(notia_backend_core::DocumentKind::Text),
                        _ => None,
                    },
                    offset: call.arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                    limit: Self::page_limit(&call.arguments, "limit", 20),
                })?;
                serde_json::to_value(page).map_err(|_| invalid_result())?
            }
            "search_library_context" => {
                let page = reader.search_context(&ContextSearchRequest {
                    library_id: context.library_id.clone(),
                    query: Self::text(&call.arguments, "query"),
                    document_ids: Self::ids(&call.arguments),
                    max_results: Self::page_limit(&call.arguments, "maxResults", 20),
                })?;
                serde_json::to_value(page).map_err(|_| invalid_result())?
            }
            "search_library_exact" => {
                let page = reader.search_exact(&ExactSearchRequest {
                    library_id: context.library_id.clone(),
                    query: Self::text(&call.arguments, "query"),
                    document_ids: Self::ids(&call.arguments),
                    case_sensitive: call
                        .arguments
                        .get("caseSensitive")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    max_results: Self::page_limit(&call.arguments, "maxResults", 20),
                })?;
                serde_json::to_value(page).map_err(|_| invalid_result())?
            }
            "get_document_metadata" => {
                // Several documents at once, read now from their files.
                let ids = Self::ids(&call.arguments);
                if ids.is_empty() {
                    let locator = self.existing_document_locator(context, &call.arguments)?;
                    serde_json::to_value(reader.metadata(&locator)?).map_err(|_| invalid_result())?
                } else {
                    if ids.len() > notia_backend_core::MAX_READ_DOCUMENTS * 3 {
                        return Err(BackendError::invalid_input(format!(
                            "get_document_metadata lee hasta {} documentos por vez; para una carpeta entera usá inspect_page_chain.",
                            notia_backend_core::MAX_READ_DOCUMENTS * 3
                        )));
                    }
                    let documents = ids
                        .iter()
                        .map(|id| {
                            let arguments = json!({ "documentId": id });
                            match self.existing_document_locator(context, &arguments).and_then(|locator| reader.metadata(&locator)) {
                                Ok(metadata) => serde_json::to_value(metadata).map_err(|_| invalid_result()),
                                Err(error) => Ok(json!({ "documentId": id, "error": error.message })),
                            }
                        })
                        .collect::<Result<Vec<_>, BackendError>>()?;
                    json!({ "documents": documents })
                }
            }
            "inspect_page_chain" => {
                let folder = Self::text(&call.arguments, "folder").trim().trim_matches('/').to_string();
                if folder.is_empty() {
                    return Err(BackendError::invalid_input("inspect_page_chain necesita folder: la carpeta de las páginas (por ejemplo «Cursos/Rust»)."));
                }
                let limit = notia_backend_core::page_links::MAX_CHAIN_DOCUMENTS;
                let page = reader.inventory(&notia_backend_core::InventoryRequest {
                    library_id: context.library_id.clone(),
                    prefix: Some(notia_backend_core::LogicalPathDto::new(&folder)?),
                    offset: 0,
                    limit: limit + 1,
                })?;
                if page.total > limit {
                    return Err(BackendError::invalid_input(format!(
                        "La carpeta {folder} tiene {} documentos y se revisan hasta {limit} por vez: elegí una subcarpeta.",
                        page.total
                    )));
                }
                let exists = |path: &str| {
                    DocumentLocatorDto::new(&context.library_id, path, None, None)
                        .and_then(|locator| reader.metadata(&locator))
                        .is_ok()
                };
                let report = notia_backend_core::page_links::inspect_page_chain(&folder, page.items, &exists);
                serde_json::to_value(report).map_err(|_| invalid_result())?
            }
            "find_document_references" => {
                let locator = self.existing_document_locator(context, &call.arguments)?;
                let references = reader.references(
                    &locator,
                    Self::page_limit(&call.arguments, "maxResults", 50),
                )?;
                serde_json::to_value(references).map_err(|_| invalid_result())?
            }
            "compare_documents" => {
                let left_document_id = Self::text(&call.arguments, "leftDocumentId");
                let right_document_id = Self::text(&call.arguments, "rightDocumentId");
                if left_document_id.is_empty() || right_document_id.is_empty() {
                    return Err(BackendError::invalid_input(
                        "La comparación necesita dos documentId.",
                    ));
                }
                let comparison = reader.compare(&CompareDocumentsRequest {
                    library_id: context.library_id.clone(),
                    left_document_id,
                    right_document_id,
                })?;
                serde_json::to_value(comparison).map_err(|_| invalid_result())?
            }
            "preview_markdown_edit" => self.preview_markdown_tool(context, call)?,
            "apply_markdown_edit" => self.apply_markdown_tool(context, call)?,
            "apply_multi_document_markdown_edit" => self.apply_multi_markdown_tool(context, call)?,
            "reindex_changed_documents" => {
                let (indexed, generation) =
                    crate::library_inventory::reindex_library(&self.app, &context.library_id)?;
                json!({ "ok": true, "indexed": indexed, "generation": generation })
            }
            "preview_multi_document_markdown_edit" => {
                self.preview_multi_markdown_tool(context, call)?
            }
            "export_document" => {
                let format = match Self::text(&call.arguments, "format").to_ascii_lowercase().as_str() {
                    "pdf" => crate::backend::ExportFormat::Pdf,
                    "docx" => crate::backend::ExportFormat::Docx,
                    _ => return Err(BackendError::invalid_input("El formato de exportación no es válido.")),
                };
                let registry = self.app.state::<LibraryBindingRegistry>();
                let picker = self.app.state::<AndroidDirectoryPickerState>();
                let receipt = crate::filesystem::adapter::export_library_document(
                    registry.inner(),
                    picker.inner(),
                    &context.library_id,
                    &Self::text(&call.arguments, "path"),
                    format,
                    &crate::device_preferences::page_geometry(&self.app),
                    // No editor draws the diagrams here: they stay code.
                    &crate::backend::DiagramRenders::default(),
                )?;
                json!({
                    "destinationLogicalPath": receipt.destination_logical_path,
                    "byteLength": receipt.byte_length,
                    "contentFingerprint": receipt.content_fingerprint,
                })
            }
            "get_finance_dollar_quotes" => {
                let result = crate::host::async_runtime::block_on(
                    crate::services::finance_external::dollar_quotes(),
                )
                .map_err(|message| BackendError::new(
                    BackendErrorCode::ProviderUnavailable,
                    message,
                    true,
                ))?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "get_finance_inflation_indices" => {
                let result = crate::host::async_runtime::block_on(
                    crate::services::finance_external::inflation_indices(),
                )
                .map_err(|message| BackendError::new(
                    BackendErrorCode::ProviderUnavailable,
                    message,
                    true,
                ))?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "get_finance_historical_dollar_quotes" => {
                let from = Self::finance_date(&call.arguments, "from")?;
                let to = Self::finance_date(&call.arguments, "to")?;
                let result = crate::host::async_runtime::block_on(
                    crate::services::finance_external::historical_dollar_quotes(
                        from.as_deref(),
                        to.as_deref(),
                    ),
                )
                .map_err(|message| BackendError::new(
                    BackendErrorCode::ProviderUnavailable,
                    message,
                    true,
                ))?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "extract_finance_document" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "artifactId": Self::text(&call.arguments, "artifactId"),
                    "filePath": Self::text(&call.arguments, "filePath"),
                    "documentType": Self::text(&call.arguments, "documentType"),
                });
                let payload = serde_json::from_value::<crate::services::finance_extraction::ExtractFinanceDocumentPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("La extracción financiera no es válida."))?;
                let result = crate::host::async_runtime::block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        crate::services::finance_extraction::extract_finance_document(
                            self.app.clone(),
                            payload,
                        ),
                    )
                    .await
                    .map_err(|_| "La extracción financiera agotó el tiempo de espera.".to_string())?
                })
                .map_err(Self::finance_error)?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "get_finance_full_snapshot" => {
                let month = Self::finance_period(&call.arguments, "month", true)?
                    .ok_or_else(|| BackendError::invalid_input("El snapshot financiero necesita un mes."))?;
                let finance_context = self.finance_context(context)?;
                let dashboard = crate::finance::finance_get_dashboard(
                    self.app.clone(),
                    finance_context,
                    month.clone(),
                ).map_err(Self::finance_error)?;
                let all_movements = crate::finance::finance_list_all_transactions(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let all_savings_movements = crate::finance::finance_list_all_savings_movements(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let services = crate::finance::finance_list_services(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let service_occurrences = crate::finance::finance_list_all_service_occurrences(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let service_occurrence_versions = crate::finance::finance_list_all_service_occurrence_versions(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let service_invoices = crate::finance::finance_list_service_invoices(
                    self.app.clone(),
                    self.finance_context(context)?,
                    None,
                ).map_err(Self::finance_error)?;
                let purchases = crate::finance_records::finance_list_purchases(
                    self.app.clone(),
                    crate::finance_records::ListPeriodPayload {
                        context: self.finance_context(context)?,
                        from: None,
                        to: None,
                        merchant_id: None,
                        product_id: None,
                    },
                ).map_err(Self::finance_error)?;
                let salaries = crate::finance_records::finance_list_salaries(
                    self.app.clone(),
                    crate::finance_records::ListPeriodPayload {
                        context: self.finance_context(context)?,
                        from: None,
                        to: None,
                        merchant_id: None,
                        product_id: None,
                    },
                ).map_err(Self::finance_error)?;
                let statements = crate::finance_records::finance_list_credit_card_statements(
                    self.app.clone(),
                    crate::finance_records::ListPeriodPayload {
                        context: self.finance_context(context)?,
                        from: None,
                        to: None,
                        merchant_id: None,
                        product_id: None,
                    },
                ).map_err(Self::finance_error)?;
                let price_history = crate::finance_records::finance_list_price_history(
                    self.app.clone(),
                    crate::finance_records::ListPeriodPayload {
                        context: self.finance_context(context)?,
                        from: None,
                        to: None,
                        merchant_id: None,
                        product_id: None,
                    },
                ).map_err(Self::finance_error)?;
                let installment_plans = crate::finance_records::finance_list_installment_plans(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                let installments = crate::finance_records::finance_list_installments(
                    self.app.clone(),
                    crate::finance_records::ListInstallmentsPayload {
                        context: self.finance_context(context)?,
                        plan_id: None,
                    },
                ).map_err(Self::finance_error)?;
                let review_items = crate::finance_edits::finance_list_review_items(
                    self.app.clone(),
                    self.finance_context(context)?,
                    None,
                ).map_err(Self::finance_error)?;
                let artifacts = crate::services::finance_extraction::list_finance_artifacts(
                    self.app.clone(),
                    self.finance_context(context)?,
                ).map_err(Self::finance_error)?;
                json!({
                    "ok": true,
                    "month": month,
                    "dashboard": dashboard,
                    "allMovements": all_movements,
                    "allSavingsMovements": all_savings_movements,
                    "services": services,
                    "serviceOccurrences": service_occurrences,
                    "serviceOccurrenceVersions": service_occurrence_versions,
                    "serviceInvoices": service_invoices,
                    "purchases": purchases,
                    "salaries": salaries,
                    "creditCardStatements": statements,
                    "priceHistory": price_history,
                    "installmentPlans": installment_plans,
                    "installments": installments,
                    "reviewItems": review_items,
                    "artifacts": artifacts,
                })
            }
            "get_finance_dashboard" => {
                let month = Self::finance_period(&call.arguments, "month", true)?
                    .ok_or_else(|| BackendError::invalid_input("El dashboard financiero necesita un mes."))?;
                let result = crate::finance::finance_get_dashboard(
                    self.app.clone(),
                    self.finance_context(context)?,
                    month,
                )
                .map_err(Self::finance_error)?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "add_agent_memory" => self.append_agent_file(
                context,
                crate::agent_workspace::AgentItem::Memory,
                &Self::first_text(&call.arguments, "memory", "content"),
            )?,
            "add_agent_rule" => self.append_agent_file(
                context,
                crate::agent_workspace::AgentItem::Rule,
                &Self::text(&call.arguments, "rule"),
            )?,
            "add_agent_thought" => self.append_agent_file(
                context,
                crate::agent_workspace::AgentItem::Thought,
                &Self::first_text(&call.arguments, "thought", "content"),
            )?,
            "add_agent_biography" => self.append_agent_file(
                context,
                crate::agent_workspace::AgentItem::Biography,
                &Self::first_text(&call.arguments, "entry", "content"),
            )?,
            "add_agent_talk" => self.append_agent_file(
                context,
                crate::agent_workspace::AgentItem::Talk,
                &Self::first_text(&call.arguments, "observation", "content"),
            )?,
            "add_agent_person" => {
                let entry = notia_backend_core::agent_workspace::person_entry(
                    &Self::text(&call.arguments, "name"),
                    &Self::first_text(&call.arguments, "about", "content"),
                )
                .ok_or_else(|| BackendError::invalid_input("Indicá el nombre de la persona y quién es, en una oración breve."))?;
                self.append_agent_file(context, crate::agent_workspace::AgentItem::Person, &entry)?
            }
            "add_agent_improvement" => {
                Self::ensure_agent_memory_writable(context)?;
                let kind = notia_backend_core::agent_workspace::ImprovementKind::parse(&Self::text(&call.arguments, "kind"));
                let text = Self::first_text(&call.arguments, "improvement", "content");
                json!({ "changed": crate::agent_workspace::append_improvement(&self.app, &context.library_id, kind, &text)? })
            }
            "get_finance_record" if !matches!(Self::text(&call.arguments, "entity").as_str(), "" | "movements") => {
                self.finance_record_by_id(context, &Self::text(&call.arguments, "entity"), &Self::finance_id(&call.arguments, "id")?)?
            }
            "get_finance_record" => {
                let result = crate::finance::finance_get_transaction(
                    self.app.clone(),
                    crate::finance::GetFinanceRecordPayload {
                        context: self.finance_context(context)?,
                        id: Self::finance_id(&call.arguments, "id")?,
                    },
                )
                .map_err(Self::finance_error)?;
                serde_json::to_value(result).map_err(|_| invalid_result())?
            }
            "list_finance_accounts" => serde_json::to_value(
                crate::finance::finance_list_accounts(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?,
            )
            .map_err(|_| invalid_result())?,
            "list_finance_categories" => serde_json::to_value(
                crate::finance::finance_list_categories(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?,
            )
            .map_err(|_| invalid_result())?,
            "search_finance_categories" => {
                let query = Self::text(&call.arguments, "query").to_lowercase();
                let categories = crate::finance::finance_list_categories(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?;
                serde_json::to_value(categories.into_iter().filter(|category| {
                    query.is_empty()
                        || category.name.to_lowercase().contains(&query)
                        || category
                            .description
                            .as_deref()
                            .is_some_and(|description| description.to_lowercase().contains(&query))
                }).collect::<Vec<_>>()).map_err(|_| invalid_result())?
            }
            "list_finance_movements" => {
                let month = Self::finance_period(&call.arguments, "month", true)?
                    .ok_or_else(|| BackendError::invalid_input("La lista de movimientos necesita un mes."))?;
                let result = crate::finance::finance_get_dashboard(
                    self.app.clone(),
                    self.finance_context(context)?,
                    month.clone(),
                )
                .map_err(Self::finance_error)?;
                json!({ "month": month, "movements": result.transactions })
            }
            "list_finance_records" => {
                let entity = Self::text(&call.arguments, "entity");
                let items = match entity.as_str() {
                    "accounts" | "categories" | "movements" => {
                        let month = Self::finance_period(&call.arguments, "month", true)?
                            .ok_or_else(|| BackendError::invalid_input("La consulta financiera necesita un mes."))?;
                        let dashboard = crate::finance::finance_get_dashboard(
                            self.app.clone(),
                            self.finance_context(context)?,
                            month,
                        )
                        .map_err(Self::finance_error)?;
                        match entity.as_str() {
                            "accounts" => serde_json::to_value(dashboard.accounts),
                            "categories" => serde_json::to_value(dashboard.categories),
                            _ => serde_json::to_value(dashboard.transactions),
                        }
                    }
                    "services" => serde_json::to_value(crate::finance::finance_list_services(
                        self.app.clone(),
                        self.finance_context(context)?,
                    ).map_err(Self::finance_error)?),
                    "service_occurrences" => {
                        if let Some(period) = Self::finance_period(&call.arguments, "period", false)? {
                            serde_json::to_value(crate::finance::finance_list_service_occurrences(
                                self.app.clone(),
                                self.finance_context(context)?,
                                period,
                            ).map_err(Self::finance_error)?)
                        } else {
                            serde_json::to_value(crate::finance::finance_list_all_service_occurrences(
                                self.app.clone(),
                                self.finance_context(context)?,
                            ).map_err(Self::finance_error)?)
                        }
                    }
                    "service_invoices" => serde_json::to_value(crate::finance::finance_list_service_invoices(
                        self.app.clone(),
                        self.finance_context(context)?,
                        Self::finance_period(&call.arguments, "period", false)?,
                    ).map_err(Self::finance_error)?),
                    _ => return Err(BackendError::invalid_input("La entidad financiera no está disponible en el backend.")),
                }
                .map_err(|_| invalid_result())?;
                let items = items.as_array().cloned().unwrap_or_default();
                let offset = call.arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let limit = Self::page_limit(&call.arguments, "limit", 50);
                let total = items.len();
                json!({
                    "ok": true,
                    "entity": entity,
                    "items": items.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),
                    "total": total,
                    "offset": offset,
                    "limit": limit,
                    "hasMore": offset.saturating_add(limit) < total,
                })
            }
            "list_finance_services" => {
                let result = crate::finance::finance_list_services(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?;
                json!({ "services": result })
            }
            "list_finance_service_occurrences" => {
                let period = Self::finance_period(&call.arguments, "period", true)?
                    .ok_or_else(|| BackendError::invalid_input("La lista de ocurrencias necesita un período."))?;
                let result = crate::finance::finance_list_service_occurrences(
                    self.app.clone(),
                    self.finance_context(context)?,
                    period.clone(),
                )
                .map_err(Self::finance_error)?;
                json!({ "period": period, "occurrences": result })
            }
            "list_finance_service_invoices" => {
                let period = Self::finance_period(&call.arguments, "period", false)?;
                let result = crate::finance::finance_list_service_invoices(
                    self.app.clone(),
                    self.finance_context(context)?,
                    period.clone(),
                )
                .map_err(Self::finance_error)?;
                json!({ "period": period, "invoices": result })
            }
            "list_finance_artifacts" => serde_json::to_value(
                crate::services::finance_extraction::list_finance_artifacts(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?,
            )
            .map_err(|_| invalid_result())?,
            "list_finance_review_items" => {
                let status = Self::optional_text(&call.arguments, "status");
                let items = crate::finance_edits::finance_list_review_items(
                    self.app.clone(),
                    self.finance_context(context)?,
                    status,
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "reviewItems": items })
            }
            "list_finance_products" | "list_finance_merchants" => {
                let payload = crate::finance_records::ListCatalogPayload {
                    context: self.finance_context(context)?,
                    search: Self::optional_text(&call.arguments, "search"),
                    limit: Some(Self::page_limit(&call.arguments, "limit", 50) as u32),
                };
                if call.name == "list_finance_products" {
                    let products = crate::finance_records::finance_list_products(self.app.clone(), payload)
                        .map_err(Self::finance_error)?;
                    json!({ "ok": true, "products": products })
                } else {
                    let merchants = crate::finance_records::finance_list_merchants(self.app.clone(), payload)
                        .map_err(Self::finance_error)?;
                    json!({ "ok": true, "merchants": merchants })
                }
            }
            "list_finance_installment_plans" => serde_json::to_value(
                crate::finance_records::finance_list_installment_plans(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?,
            ).map_err(|_| invalid_result())?,
            "list_finance_installments" => {
                let plan_id = call
                    .arguments
                    .get("planId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                serde_json::to_value(crate::finance_records::finance_list_installments(
                    self.app.clone(),
                    crate::finance_records::ListInstallmentsPayload {
                        context: self.finance_context(context)?,
                        plan_id,
                    },
                ).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "list_finance_purchases" | "list_finance_salaries" | "list_finance_price_history"
            | "list_finance_credit_card_statements" => {
                let payload = crate::finance_records::ListPeriodPayload {
                    context: self.finance_context(context)?,
                    from: Self::optional_text(&call.arguments, "from"),
                    to: Self::optional_text(&call.arguments, "to"),
                    merchant_id: Self::optional_text(&call.arguments, "merchantId"),
                    product_id: Self::optional_text(&call.arguments, "productId"),
                };
                let result = match call.name.as_str() {
                    "list_finance_purchases" => serde_json::to_value(
                        crate::finance_records::finance_list_purchases(self.app.clone(), payload)
                            .map_err(Self::finance_error)?,
                    ),
                    "list_finance_salaries" => serde_json::to_value(
                        crate::finance_records::finance_list_salaries(self.app.clone(), payload)
                            .map_err(Self::finance_error)?,
                    ),
                    "list_finance_price_history" => serde_json::to_value(
                        crate::finance_records::finance_list_price_history(self.app.clone(), payload)
                            .map_err(Self::finance_error)?,
                    ),
                    "list_finance_credit_card_statements" => serde_json::to_value(
                        crate::finance_records::finance_list_credit_card_statements(
                            self.app.clone(),
                            payload,
                        )
                        .map_err(Self::finance_error)?,
                    ),
                    _ => unreachable!("finance list tool was checked above"),
                };
                result.map_err(|_| invalid_result())?
            }
            "save_finance_account" => {
                let payload = self.finance_save_payload::<crate::finance::SaveAccountPayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_account(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_category" => {
                let finance_context = self.finance_context(context)?;
                let dashboard = self.finance_dashboard(&finance_context)?;
                match finance_agent_inputs::build_category(&call.arguments, &dashboard.categories) {
                    Err(error) => error.to_value(),
                    Ok(finance_agent_inputs::CreateOrExisting::Existing(category)) => {
                        json!({ "ok": true, "changed": false, "duplicate": true, "category": category })
                    }
                    Ok(finance_agent_inputs::CreateOrExisting::Create(category)) => {
                        let saved = crate::finance::finance_save_category(
                            self.app.clone(),
                            crate::finance::SaveCategoryPayload { context: finance_context, category },
                        )
                        .map_err(Self::finance_error)?;
                        json!({ "ok": true, "changed": true, "category": saved })
                    }
                }
            }
            "save_finance_category" => {
                let payload = self.finance_save_payload::<crate::finance::SaveCategoryPayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_category(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_transaction" => {
                let payload = self.finance_save_payload::<crate::finance::SaveTransactionPayload>(context, call)?;
                let (transaction, outcome) = crate::finance::finance_save_transaction_linked(self.app.clone(), payload)
                    .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "transaction": transaction, "links": outcome.links, "reviewItems": outcome.review_items })
            }
            "create_finance_transaction" => self.create_finance_transaction(context, call)?,
            "create_finance_service" => {
                let finance_context = self.finance_context(context)?;
                let dashboard = self.finance_dashboard(&finance_context)?;
                let services = crate::finance::finance_list_services(self.app.clone(), finance_context.clone())
                    .map_err(Self::finance_error)?;
                match finance_agent_inputs::build_service(&call.arguments, &dashboard.categories, &services) {
                    Err(error) => error.to_value(),
                    Ok(finance_agent_inputs::CreateOrExisting::Existing(service)) => {
                        json!({ "ok": true, "changed": false, "duplicate": true, "service": service })
                    }
                    Ok(finance_agent_inputs::CreateOrExisting::Create(service)) => {
                        let saved = crate::finance::finance_save_service(
                            self.app.clone(),
                            crate::finance::SaveFinanceServicePayload { context: finance_context, service },
                        )
                        .map_err(Self::finance_error)?;
                        json!({ "ok": true, "changed": true, "service": saved })
                    }
                }
            }
            "save_finance_service" => {
                let payload = self.finance_save_payload::<crate::finance::SaveFinanceServicePayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_service(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_service_occurrence" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "occurrence": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "serviceId": Self::text(&call.arguments, "serviceId"),
                        "period": Self::text(&call.arguments, "period"),
                        "expectedAmount": Self::text(&call.arguments, "expectedAmount"),
                        "paidAmount": Self::optional_text(&call.arguments, "paidAmount"),
                        "effectiveDate": Self::optional_text(&call.arguments, "effectiveDate"),
                        "status": if Self::text(&call.arguments, "status") == "accepted" {
                            "accepted"
                        } else if Self::optional_text(&call.arguments, "paidAmount").is_some() {
                            "current"
                        } else {
                            "pending"
                        },
                        "transactionId": Self::optional_text(&call.arguments, "transactionId"),
                        "artifactId": Self::optional_text(&call.arguments, "artifactId"),
                        "sourceReference": Self::optional_text(&call.arguments, "sourceReference"),
                        "rawSource": Self::optional_text(&call.arguments, "rawSource"),
                        "actorLibraryUserId": context.actor.library_user_id.clone(),
                        "source": "backend",
                        "createdAt": Value::Null,
                        "updatedAt": Value::Null,
                    },
                    "reason": Self::optional_text(&call.arguments, "reason"),
                });
                let payload = serde_json::from_value::<crate::finance::SaveFinanceServiceOccurrencePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("La ocurrencia financiera no es válida."))?;
                serde_json::to_value(crate::finance::finance_save_service_occurrence(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_service_occurrence" => {
                let payload = self.finance_save_payload::<crate::finance::SaveFinanceServiceOccurrencePayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_service_occurrence(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_service_invoice" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "invoice": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "serviceId": Self::optional_text(&call.arguments, "serviceId"),
                        "period": Self::text(&call.arguments, "period"),
                        "dueDate": Self::optional_text(&call.arguments, "dueDate"),
                        "provider": Self::optional_text(&call.arguments, "provider"),
                        "amount": Self::text(&call.arguments, "amount"),
                        "currency": Self::text(&call.arguments, "currency"),
                        "transactionId": Self::optional_text(&call.arguments, "transactionId"),
                        "artifactId": Self::optional_text(&call.arguments, "artifactId"),
                        "validationStatus": "pending",
                        "sourceReference": Self::optional_text(&call.arguments, "sourceReference"),
                        "rawExtraction": Self::optional_text(&call.arguments, "rawExtraction"),
                        "createdAt": Value::Null,
                        "updatedAt": Value::Null,
                    },
                });
                let payload = serde_json::from_value::<crate::finance::SaveFinanceServiceInvoicePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("La factura financiera no es válida."))?;
                serde_json::to_value(crate::finance::finance_save_service_invoice(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_service_invoice" => {
                let payload = self.finance_save_payload::<crate::finance::SaveFinanceServiceInvoicePayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_service_invoice(self.app.clone(), payload).map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "set_finance_service_active" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "id": Self::text(&call.arguments, "serviceId"),
                    "active": call.arguments.get("active").and_then(Value::as_bool).unwrap_or(false),
                });
                let payload = serde_json::from_value::<crate::finance::SetFinanceServiceActivePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El estado del servicio financiero no es válido."))?;
                crate::finance::finance_set_service_active(self.app.clone(), payload)
                    .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true })
            }
            "link_finance_savings_account" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "reserveId": Self::text(&call.arguments, "reserveId"),
                    "accountId": Self::text(&call.arguments, "accountId"),
                });
                let payload = serde_json::from_value::<crate::finance::LinkSavingsAccountPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El vínculo de ahorro no es válido."))?;
                crate::finance::finance_link_savings_account(self.app.clone(), payload)
                    .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true })
            }
            "delete_finance_record" => {
                let entity = Self::text(&call.arguments, "entity");
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "id": Self::text(&call.arguments, "id"),
                });
                let payload = serde_json::from_value::<crate::finance::DeleteFinanceEntityPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El registro financiero no es válido."))?;
                let summary = match entity.as_str() {
                    "transaction" => crate::finance::finance_delete_transaction(self.app.clone(), payload).map(|_| None),
                    "account" => crate::finance::finance_delete_account(self.app.clone(), payload).map(|_| None),
                    "category" => crate::finance::finance_delete_category(self.app.clone(), payload).map(|_| None),
                    other if crate::finance_edits::DELETABLE_RECORDS.contains(&other) => {
                        crate::finance_edits::finance_delete_record(
                            self.app.clone(),
                            payload.context,
                            entity.clone(),
                            payload.id,
                        )
                        .map(|deleted| Some(deleted.summary))
                    }
                    _ => return Err(BackendError::invalid_input("La entidad financiera no se puede eliminar.")),
                }
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "entity": entity, "summary": summary })
            }
            "clear_finance_data" => {
                crate::finance::finance_clear_all_data(
                    self.app.clone(),
                    self.finance_context(context)?,
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "scope": "all-finance-data" })
            }
            "reverse_finance_transaction" => {
                let id = Self::text(&call.arguments, "transactionId");
                let reason = Self::text(&call.arguments, "reason");
                if id.is_empty() || reason.is_empty() {
                    return Err(BackendError::invalid_input("La reversión necesita movimiento y motivo."));
                }
                let context_for_read = self.finance_context(context)?;
                let mut transaction = crate::finance::finance_get_transaction(
                    self.app.clone(),
                    crate::finance::GetFinanceRecordPayload {
                        context: context_for_read.clone(),
                        id: id.clone(),
                    },
                ).map_err(Self::finance_error)?;
                transaction.status = "discarded".to_string();
                let saved = crate::finance::finance_save_transaction(
                    self.app.clone(),
                    crate::finance::SaveTransactionPayload {
                        context: context_for_read,
                        transaction,
                    },
                ).map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "transactionId": id, "reason": reason, "record": saved })
            }
            "update_finance_transaction_status" => {
                let id = Self::text(&call.arguments, "transactionId");
                let status = Self::text(&call.arguments, "status");
                if id.is_empty() || status.is_empty() {
                    return Err(BackendError::invalid_input("La actualización necesita movimiento y estado."));
                }
                let finance_context = self.finance_context(context)?;
                let mut transaction = crate::finance::finance_get_transaction(
                    self.app.clone(),
                    crate::finance::GetFinanceRecordPayload {
                        context: finance_context.clone(),
                        id: id.clone(),
                    },
                ).map_err(Self::finance_error)?;
                transaction.status = status.clone();
                let saved = crate::finance::finance_save_transaction(
                    self.app.clone(),
                    crate::finance::SaveTransactionPayload {
                        context: finance_context,
                        transaction,
                    },
                ).map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "transactionId": id, "status": status, "record": saved })
            }
            "save_finance_savings_reserve" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "reserve": record });
                let payload = serde_json::from_value::<crate::finance::SaveSavingsReservePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("La reserva de ahorro no es válida."))?;
                serde_json::to_value(crate::finance::finance_save_savings_reserve(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_savings_movement" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "movement": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "reserveId": Self::text(&call.arguments, "reserveId"),
                        "accountId": Self::optional_text(&call.arguments, "accountId"),
                        "movementType": Self::text(&call.arguments, "movementType"),
                        "amount": Self::text(&call.arguments, "amount"),
                        "currency": Self::text(&call.arguments, "currency"),
                        "effectiveDate": Self::text(&call.arguments, "effectiveDate"),
                        "description": Self::text(&call.arguments, "description"),
                        "reason": Self::optional_text(&call.arguments, "reason"),
                        "source": "backend",
                        "status": "confirmed",
                        "actorUserId": Value::Null,
                        "actorLibraryUserId": context.actor.library_user_id.clone(),
                        "linkedTransactionId": Self::optional_text(&call.arguments, "linkedTransactionId"),
                    },
                });
                let payload = serde_json::from_value::<crate::finance::SaveSavingsMovementPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El movimiento de ahorro no es válido."))?;
                serde_json::to_value(crate::finance::finance_save_savings_movement(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_savings_movement" => {
                let payload = self.finance_save_payload::<crate::finance::SaveSavingsMovementPayload>(context, call)?;
                serde_json::to_value(crate::finance::finance_save_savings_movement(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_savings_exchange" => {
                let payload = serde_json::json!({
                    "context": self.finance_context(context)?,
                    "exchange": {
                        "id": uuid::Uuid::new_v4().to_string(),
                        "reserveId": Self::first_text(&call.arguments, "reserveId", "reserve"),
                        "sourceAccountId": Self::first_text(&call.arguments, "sourceAccountId", "sourceAccount"),
                        "sourceAmount": Self::text(&call.arguments, "sourceAmount"),
                        "sourceCurrency": Self::text(&call.arguments, "sourceCurrency"),
                        "savingsAmount": Self::text(&call.arguments, "savingsAmount"),
                        "savingsCurrency": Self::text(&call.arguments, "savingsCurrency"),
                        "effectiveDate": Self::text(&call.arguments, "effectiveDate"),
                        "description": Self::text(&call.arguments, "description"),
                        "actorUserId": Value::Null,
                        "sourceReference": Self::optional_text(&call.arguments, "sourceReference"),
                        "rawSource": Self::optional_text(&call.arguments, "rawSource"),
                        "direction": Self::optional_text(&call.arguments, "direction"),
                    },
                });
                let payload = serde_json::from_value::<crate::finance::SaveSavingsExchangePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El intercambio de ahorro no es válido."))?;
                serde_json::to_value(crate::finance::finance_save_savings_exchange(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_savings_exchange" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "exchange": record });
                let payload = serde_json::from_value::<crate::finance::SaveSavingsExchangePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El intercambio de ahorro no es válido."))?;
                serde_json::to_value(crate::finance::finance_save_savings_exchange(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_purchase" => {
                let finance_context = self.finance_context(context)?;
                let dashboard = self.finance_dashboard(&finance_context)?;
                let services = crate::finance::finance_list_services(self.app.clone(), finance_context.clone())
                    .map_err(Self::finance_error)?;
                match finance_agent_inputs::build_purchase(
                    &call.arguments,
                    &dashboard.accounts,
                    &dashboard.categories,
                    &services,
                    &Self::agent_source_reference(context, call),
                    &utc_today(),
                ) {
                    Err(error) => error.to_value(),
                    Ok(purchase) => {
                        let saved = crate::finance_records::finance_save_purchase(
                            self.app.clone(),
                            crate::finance_records::SavePurchasePayload { context: finance_context, purchase },
                        )
                        .map_err(Self::finance_error)?;
                        json!({ "ok": true, "changed": true, "purchase": saved })
                    }
                }
            }
            "save_finance_purchase" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "purchase": record });
                let payload = serde_json::from_value::<crate::finance_records::SavePurchasePayload>(payload)
                    .map_err(|_| BackendError::invalid_input("La compra financiera no es válida."))?;
                serde_json::to_value(crate::finance_records::finance_save_purchase(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_salary" => {
                let finance_context = self.finance_context(context)?;
                let dashboard = self.finance_dashboard(&finance_context)?;
                match finance_agent_inputs::build_salary(
                    &call.arguments,
                    &dashboard.accounts,
                    &Self::agent_source_reference(context, call),
                ) {
                    Err(error) => error.to_value(),
                    Ok(salary) => {
                        let saved = crate::finance_records::finance_save_salary(
                            self.app.clone(),
                            crate::finance_records::SaveSalaryPayload { context: finance_context, salary },
                        )
                        .map_err(Self::finance_error)?;
                        json!({ "ok": true, "changed": true, "salary": saved })
                    }
                }
            }
            "save_finance_salary" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "salary": record });
                let payload = serde_json::from_value::<crate::finance_records::SaveSalaryPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El sueldo financiero no es válido."))?;
                serde_json::to_value(crate::finance_records::finance_save_salary(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "create_finance_credit_card_statement" => {
                let finance_context = self.finance_context(context)?;
                let dashboard = self.finance_dashboard(&finance_context)?;
                match finance_agent_inputs::build_credit_card_statement(
                    &call.arguments,
                    &dashboard.accounts,
                    &Self::agent_source_reference(context, call),
                ) {
                    Err(error) => error.to_value(),
                    Ok(statement) => {
                        let saved = crate::finance_records::finance_save_credit_card_statement(
                            self.app.clone(),
                            crate::finance_records::SaveCreditCardStatementPayload { context: finance_context, statement },
                        )
                        .map_err(Self::finance_error)?;
                        json!({ "ok": true, "changed": true, "statement": saved })
                    }
                }
            }
            "save_finance_credit_card_statement" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "statement": record });
                let payload = serde_json::from_value::<crate::finance_records::SaveCreditCardStatementPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El resumen de tarjeta no es válido."))?;
                serde_json::to_value(crate::finance_records::finance_save_credit_card_statement(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "save_finance_installment_plan" => {
                let record = Self::finance_record(&call.arguments);
                let payload = serde_json::json!({ "context": self.finance_context(context)?, "plan": record });
                let payload = serde_json::from_value::<crate::finance_records::SaveInstallmentPayload>(payload)
                    .map_err(|_| BackendError::invalid_input("El plan de cuotas no es válido."))?;
                serde_json::to_value(crate::finance_records::finance_save_installment_plan(self.app.clone(), payload)
                    .map_err(Self::finance_error)?).map_err(|_| invalid_result())?
            }
            "link_finance_records" => {
                let link = crate::finance_edits::finance_link_records(
                    self.app.clone(),
                    self.finance_context(context)?,
                    Self::text(&call.arguments, "kind"),
                    Self::text(&call.arguments, "subjectId"),
                    Self::text(&call.arguments, "lineId"),
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "link": link })
            }
            "unlink_finance_records" => {
                crate::finance_edits::finance_unlink_records(
                    self.app.clone(),
                    self.finance_context(context)?,
                    Self::optional_text(&call.arguments, "linkId"),
                    Self::optional_text(&call.arguments, "lineId"),
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true })
            }
            "resolve_finance_review_item" => {
                let item = crate::finance_edits::finance_resolve_review_item(
                    self.app.clone(),
                    self.finance_context(context)?,
                    Self::text(&call.arguments, "reviewItemId"),
                    Self::text(&call.arguments, "optionId"),
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true, "reviewItem": item })
            }
            "rename_finance_product" | "rename_finance_merchant" | "merge_finance_products" | "merge_finance_merchants" => {
                let (action, value_field) = match call.name.as_str() {
                    "rename_finance_product" => ("rename-product", "name"),
                    "rename_finance_merchant" => ("rename-merchant", "name"),
                    "merge_finance_products" => ("merge-products", "intoId"),
                    _ => ("merge-merchants", "intoId"),
                };
                crate::finance_edits::finance_edit_catalog(
                    self.app.clone(),
                    self.finance_context(context)?,
                    action.to_string(),
                    Self::text(&call.arguments, "id"),
                    Self::text(&call.arguments, value_field),
                )
                .map_err(Self::finance_error)?;
                json!({ "ok": true, "changed": true })
            }
            "read_active_markdown_document" => {
                // The active document comes from the request snapshot; an
                // explicit path is only accepted when it is that document.
                let active = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active_document.as_ref())
                    .ok_or_else(|| {
                        BackendError::invalid_input("No hay un documento activo en esta vista.")
                    })?;
                // The tab may name it with backslashes or its whole folder.
                let root = self.library_location(context).ok().map(|(root, _)| root).filter(|root| !root.is_empty());
                let active_path = notia_backend_core::library_tools::workspace_logical_path(&active.path, root.as_deref())
                    .ok_or_else(|| {
                        BackendError::invalid_input(format!(
                            "El documento activo «{}» no está en la biblioteca con una ruta que Notia pueda leer: buscalo por su nombre con las herramientas de la biblioteca y leelo con read_library_documents.",
                            active.name
                        ))
                    })?
                    .as_str()
                    .to_string();
                let requested = Self::explicit_path(&call.arguments)
                    .map(|requested| notia_backend_core::library_tools::workspace_logical_path(&requested, root.as_deref()).map_or(requested, |path| path.as_str().to_string()));
                if requested.is_some_and(|requested| requested != active_path) {
                    return Err(BackendError::new(
                        BackendErrorCode::Forbidden,
                        "Solo se puede leer el documento activo con esta herramienta.",
                        false,
                    ));
                }
                let locator = DocumentLocatorDto::new(&context.library_id, &active_path, None, None)?;
                serde_json::to_value(reader.read_document(&locator)?).map_err(|_| invalid_result())?
            }
            "create_library_note" => {
                let locator = Self::new_note_locator(context, &call.arguments)?;
                Self::ensure_agent_writable(&locator)?;
                let content = Self::text(&call.arguments, "content");
                if content.is_empty() {
                    return Err(BackendError::invalid_input(
                        "La creación documental necesita contenido.",
                    ));
                }
                let content = self.with_message_images(content)?;
                let content = crate::folder_contexts::enforce(&self.app, &context.library_id, locator.logical_path.as_str(), content);
                let registry = self.app.state::<LibraryBindingRegistry>();
                let picker = self.app.state::<AndroidDirectoryPickerState>();
                let adapter = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
                    registry.inner(),
                    &context.library_id,
                    picker.inner(),
                )?;
                adapter.create_text_locator(&locator, &content)?;
                json!({ "path": locator.logical_path.as_str(), "created": true })
            }
            "replace_library_document" => {
                let locator = self.existing_document_locator(context, &call.arguments)?;
                Self::ensure_agent_writable(&locator)?;
                let previous = self.reader(&context.library_id)?.read_document(&locator)?.content;
                let (content, frontmatter) =
                    notia_backend_core::library_tools::replacement_content(&previous, &self.with_message_images(Self::text(&call.arguments, "content"))?);
                let content = crate::folder_contexts::enforce(&self.app, &context.library_id, locator.logical_path.as_str(), content);
                let sizes = (previous.chars().count(), content.chars().count());
                // Tools report backend-core revisions (numbers); the editor
                // may still send the platform `sha256:` form.
                let expected = call.arguments.get("expectedRevision");
                let core_revision = expected.and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
                });
                let platform_revision = expected
                    .and_then(Value::as_str)
                    .filter(|text| text.starts_with("sha256:"));
                let registry = self.app.state::<LibraryBindingRegistry>();
                let picker = self.app.state::<AndroidDirectoryPickerState>();
                let adapter = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
                    registry.inner(),
                    &context.library_id,
                    picker.inner(),
                )?;
                match core_revision {
                    Some(revision) => adapter.write_locator_at_revision(&locator, &content, revision)?,
                    None => adapter.write_locator(&locator, &content, platform_revision)?,
                }
                let kept = match frontmatter {
                    "kept" => "Se conservó el encabezado (frontmatter) que tenía el documento y se reemplazó el cuerpo.",
                    "replaced" => "El encabezado (frontmatter) del documento es el del contenido nuevo.",
                    _ => "El documento no tenía encabezado (frontmatter).",
                };
                json!({
                    "path": locator.logical_path.as_str(),
                    "updated": true,
                    "frontmatter": frontmatter,
                    "charactersBefore": sizes.0,
                    "charactersAfter": sizes.1,
                    "note": format!("{kept} El contenido quedó tal cual lo mandaste, sin otra conversión de formato. Para cambiar solo una parte usá preview_markdown_edit y apply_markdown_edit."),
                })
            }
            "delete_library_document" => {
                let locator = self.existing_document_locator(context, &call.arguments)?;
                Self::ensure_agent_writable(&locator)?;
                let registry = self.app.state::<LibraryBindingRegistry>();
                let picker = self.app.state::<AndroidDirectoryPickerState>();
                let adapter = crate::filesystem::adapter::TauriFilesystemDocumentAdapter::for_library(
                    registry.inner(),
                    &context.library_id,
                    picker.inner(),
                )?;
                adapter.delete_locator(&locator)?;
                json!({ "path": locator.logical_path.as_str(), "deleted": true })
            }
            "move_library_entry" => {
                let entry_move = self.entry_move(context, &call.arguments)?;
                crate::library_session::move_entry(&self.app, &context.library_id, &entry_move)?;
                json!({ "from": entry_move.source.as_str(), "path": entry_move.destination.as_str(), "moved": true })
            }
            "search_task_tickets"
            | "read_task_tickets"
            | "read_all_task_tickets"
            | "search_task_context"
            | "get_task_manager_options"
            | "get_task_board_summary"
            | "get_task_board_settings"
            | "get_task_board_plan" => {
                let task_state = self.app.state::<crate::task_manager_commands::TaskManagerBackendState>();
                let registry = self.app.state::<LibraryBindingRegistry>();
                crate::task_manager_commands::execute_backend_read_tool(
                    &self.app,
                    task_state.inner(),
                    registry.inner(),
                    &context.library_id,
                    &context.actor.library_user_id,
                    self.task_board_scope(context)?,
                    call.name.as_str(),
                    &call.arguments,
                )?
            }
            name if notia_backend_core::task_board_tool_input::is_task_board_action_tool(name) => {
                let task_state = self.app.state::<crate::task_manager_commands::TaskManagerBackendState>();
                let registry = self.app.state::<LibraryBindingRegistry>();
                crate::task_manager_commands::execute_board_action_tool(
                    &self.app,
                    task_state.inner(),
                    registry.inner(),
                    &context.library_id,
                    &context.actor.library_user_id,
                    self.task_board_scope(context)?,
                    call.name.as_str(),
                    &call.arguments,
                )?
            }
            name if notia_backend_core::is_task_mutation_tool(name) => {
                let task_state = self.app.state::<crate::task_manager_commands::TaskManagerBackendState>();
                let registry = self.app.state::<LibraryBindingRegistry>();
                crate::task_manager_commands::execute_backend_mutation_tool(
                    &self.app,
                    task_state.inner(),
                    registry.inner(),
                    &context.library_id,
                    &context.actor.library_user_id,
                    self.task_board_scope(context)?,
                    &call.id,
                    call.name.as_str(),
                    &call.arguments,
                )?
            }
            name if crate::health::is_health_tool(name) => {
                crate::health::execute_tool(&self.app, &Self::health_context(context), &call.id, name, &call.arguments, &self.request_images)
                    .map_err(crate::health::tool_error)?
            }
            name if crate::gym::is_gym_tool(name) => {
                crate::gym::execute_tool(&self.app, &Self::gym_context(context), name, &call.arguments, &self.request_images)
                    .map_err(crate::gym::tool_error)?
            }
            name if notia_backend_core::image_generation::is_image_tool(name) => self.image_tool(context, call)?,
            name if notia_backend_core::browser::is_browser_tool(name) => crate::browser::execute_tool(&self.app, &self.control, context, name, &call.arguments)?,
            name if notia_backend_core::whatsapp::is_whatsapp_tool(name) => crate::whatsapp::execute_tool(&self.app, context, name, &call.arguments)?,
            name if notia_backend_core::agent_scripts::is_script_tool(name) => crate::agent_scripts::execute_tool(&self.app, &self.control, context, name, &call.arguments)?,
            name if notia_backend_core::terminal::is_terminal_tool(name) => crate::terminal::execute_tool(
                &self.app,
                &self.control,
                self.events.clone().map(|sink| Arc::new(sink) as Arc<dyn notia_backend_core::BackendEventSink>),
                self.live_status,
                context,
                &call.id,
                &call.arguments,
            )?,
            name if crate::recipes::is_recipe_tool(name) => {
                crate::recipes::execute_tool(&self.app, &context.library_id, &call.id, name, &call.arguments, &self.request_images)
                    .map_err(crate::recipes::tool_error)?
            }
            name if crate::ai_actions::is_ai_action_tool(name) => {
                let database = self.ai_actions_database(context)?;
                crate::ai_actions::execute_tool(&self.app, &context.library_id, &database, name, &call.arguments)
                    .map_err(crate::ai_actions::tool_error)?
            }
            name if crate::ai_task_runner::is_ai_task_tool(name) => {
                crate::ai_task_runner::execute_tool(&self.app, context, name, &call.arguments)?
            }
            name if crate::routine_tools::is_routine_tool(name) => crate::routine_tools::execute_tool(
                &self.app,
                &self.routine_context(context)?,
                name,
                &call.arguments,
            )
            .map_err(Self::routine_error)?,
            name if crate::agenda_tools::is_agenda_tool(name) => {
                crate::agenda_tools::execute_tool(&self.app, &self.agenda_context(context)?, name, &call.arguments)
                    .map_err(Self::agenda_error)?
            }
            name if notia_backend_core::mail_tools::is_mail_tool(name) => crate::mail_tools::execute(&self.app, context, call)?,
            name if notia_backend_core::meeting_tools::is_meeting_tool(name) => {
                crate::meeting::execute_agent_tool(&self.app, &context.library_id, name, &call.arguments)?
            }
            "get_weather" => crate::weather::execute_tool(&self.app, &context.library_id, &call.arguments)?,
            _ => {
                return Err(BackendError::new(
                    BackendErrorCode::Unsupported,
                    "La tool todavía no tiene un adaptador backend disponible.",
                    false,
                ))
            }
        };
        if let Some((locator, previous_content)) = undo_target {
            if data.get("changed").and_then(Value::as_bool) != Some(false) {
                self.record_undo(context, call, &locator, previous_content)?;
            }
        }
        // A mutating tool reports its real outcome; validation errors,
        // duplicates and declines must not be counted as changes.
        let reported_changed = data.get("changed").and_then(Value::as_bool);
        let reported_ok = data.get("ok").and_then(Value::as_bool).unwrap_or(true);
        Ok(ToolResult {
            call_id: call.id.clone(),
            ok: reported_ok,
            changed: reported_changed.unwrap_or(reported_ok) && (crate::routine_tools::is_routine_write_tool(&call.name) || crate::ai_actions::is_ai_action_write_tool(&call.name) || crate::ai_task_runner::is_ai_task_write_tool(&call.name) || crate::recipes::is_recipe_write_tool(&call.name) || crate::health::is_health_write_tool(&call.name) || crate::gym::is_gym_write_tool(&call.name) || crate::agenda_tools::is_agenda_write_tool(&call.name) || notia_backend_core::meeting_tools::is_meeting_write_tool(&call.name) || notia_backend_core::mail_tools::is_mail_write_tool(&call.name) || notia_backend_core::terminal::is_terminal_tool(&call.name) || matches!(
                call.name.as_str(),
                "create_library_note"
                    | "replace_library_document"
                    | "delete_library_document"
                    | "move_library_entry"
                    | "export_document"
                    | "apply_markdown_edit"
                    | "apply_multi_document_markdown_edit"
                    | "add_agent_memory"
                    | "add_agent_rule"
                    | "add_agent_thought"
                    | "add_agent_biography"
                    | "add_agent_talk"
                    | "add_agent_person"
                    | "add_agent_improvement"
                    | "save_finance_account"
                    | "save_finance_category"
                    | "save_finance_transaction"
                    | "create_finance_transaction"
                    | "save_finance_service"
                    | "save_finance_service_occurrence"
                    | "save_finance_service_invoice"
                    | "create_finance_service"
                    | "create_finance_service_occurrence"
                    | "create_finance_service_invoice"
                    | "set_finance_service_active"
                    | "link_finance_savings_account"
                    | "delete_finance_record"
                    | "reverse_finance_transaction"
                    | "update_finance_transaction_status"
                    | "clear_finance_data"
                    | "extract_finance_document"
                    | "save_finance_savings_reserve"
                    | "create_finance_savings_movement"
                    | "save_finance_savings_movement"
                    | "create_finance_savings_exchange"
                    | "save_finance_savings_exchange"
                    | "save_finance_purchase"
                    | "create_finance_purchase"
                    | "save_finance_salary"
                    | "create_finance_salary"
                    | "save_finance_credit_card_statement"
                    | "create_finance_credit_card_statement"
                    | "save_finance_installment_plan"
                    | "link_finance_records"
                    | "unlink_finance_records"
                    | "resolve_finance_review_item"
                    | "rename_finance_product"
                    | "rename_finance_merchant"
                    | "merge_finance_products"
                    | "merge_finance_merchants"
            ) || notia_backend_core::is_task_mutation_tool(&call.name)
                || notia_backend_core::task_board_tool_input::is_task_board_action_tool(&call.name)),
            data: Some(data),
            error: None,
            preview: None,
        })
        .inspect(|result| {
            if result.changed && is_document_write_tool(&call.name) {
                announce_documents_changed(&self.app, &context.library_id, result.data.as_ref());
            }
            if result.changed && call.name == "move_library_entry" {
                announce_entry_moved(&self.app, &context.library_id, result.data.as_ref());
            }
        })
    }
}

pub(crate) fn execute_backend_request(
    app: &AppHandle,
    envelope: BackendRequestEnvelope,
    state: &BackendRuntimeState,
    registry: &LibraryBindingRegistry,
) -> Result<BackendResponseEnvelope, BackendError> {
    envelope.protocol_version.validate()?;
    envelope.request.validate(&BackendLimits::default())?;
    let protocol_version = envelope.protocol_version;
    let mut resume_decision: Option<(String, ResumeDecision)> = None;
    let mut request = match envelope.request {
        BackendRequest::Run(request) => request,
        BackendRequest::Resume(resume) => {
            backend_authorization_principal(
                app,
                registry,
                &resume.context.library_id,
                &resume.context.actor.library_user_id,
            )?;
            state
                .journal
                .hydrate_record(app, registry, &resume.context, &resume.idempotency_key)?;
            // A confirmed change to a note checks that the note is still as
            // the preview saw it; without the revisions every such change failed.
            let revisions = TauriRevisionPort { app: app.clone() };
            let response = state.interactions(Some(&revisions)).handle_resume(&resume)?;
            // A rejected change or plan also goes back to the agent, which
            // ends the request with its answer («La operación fue
            // rechazada.»); returning the stored interaction made the
            // clients ask the same question again.
            let should_continue = matches!(
                &response,
                BackendResponse::Resumed { status, .. }
                    if matches!(
                        status.state,
                        notia_backend_core::OperationState::Running | notia_backend_core::OperationState::Cancelled
                    )
            );
            if !should_continue {
                state.journal.clear_continuation(
                    &resume.context,
                    &resume.idempotency_key,
                    &resume.operation.operation_id,
                )?;
                state
                    .journal
                    .persist_record(app, registry, &resume.context, &resume.idempotency_key)?;
                return Ok(BackendResponseEnvelope {
                    protocol_version,
                    response,
                });
            }
            resume_decision = Some((resume.operation.operation_id.clone(), resume.decision.clone()));
            state
                .journal
                .load_request(&resume.context, &resume.idempotency_key)?
        }
        other => {
            let (context, idempotency_key) = request_identity(&other);
            backend_authorization_principal(
                app,
                registry,
                &context.library_id,
                &context.actor.library_user_id,
            )?;
            state
                .journal
                .hydrate_record(app, registry, context, idempotency_key)?;
            if let BackendRequest::GetOperation(status) = &other {
                if status.operation.is_none() {
                    if let Some(response) = state.journal.load_response(context, idempotency_key)? {
                        return Ok(BackendResponseEnvelope {
                            protocol_version,
                            response: BackendResponse::Result { response },
                        });
                    }
                }
            }
            let review_port = TauriOperationReviewPort {
                app: app.clone(),
                journal: Arc::clone(&state.journal),
            };
            let response = InteractionRuntime::new(
                state.journal.as_ref(),
                Some(&review_port),
                None,
                state.generations.as_ref(),
            )
            .with_control_registry(Arc::clone(&state.controls))
            .handle_request(&other, &BackendLimits::default())?
                .ok_or_else(|| unsupported("La operación backend no pudo ser procesada."))?;
            state
                .journal
                .persist_record(app, registry, context, idempotency_key)?;
            // A request cancelled while it waited for an answer ends here.
            if matches!(other, BackendRequest::Cancel(_)) && matches!(response, BackendResponse::Cancelled { .. }) {
                schedule_turn_reflection(app, state, context, idempotency_key, Some(CANCELLED_TURN_ENDING.to_string()));
            }
            return Ok(BackendResponseEnvelope {
                protocol_version,
                response,
            });
        }
    };
    let principal = backend_authorization_principal(
        app,
        registry,
        &request.context.library_id,
        &request.context.actor.library_user_id,
    )?;
    // A new turn with no tools of its own is routed by area; a resumed one
    // keeps the tools stored with it.
    let fresh_run = resume_decision.is_none() && request.tools.is_empty();
    let requested_tool_names = request
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    // The mail tools are offered only when the library has a connected
    // account, and the image tools when this device has the image model.
    let mail_connected = crate::mail_tools::has_connected_account(app, &request.context.library_id);
    let images_available = crate::image_generation::is_available(app);
    // The terminal and the browser only on a computer (never Android).
    let terminal_available = crate::terminal::is_available(app);
    let browser_available = crate::browser::is_available(app);
    // An AI task runs and reports by Telegram on the computer that runs the
    // library's services: elsewhere it is only read.
    let runs_services = crate::device_role::runs_services(app);
    // WhatsApp while the library's link is connected on this host.
    let whatsapp_connected = crate::whatsapp::is_connected(app, &request.context.library_id);
    let available_tool_names = TauriBackendToolExecutor::supported_tool_names()
        .iter()
        .copied()
        .filter(|name| mail_connected || !notia_backend_core::mail_tools::is_mail_tool(name))
        .filter(|name| images_available || !notia_backend_core::image_generation::is_image_tool(name))
        .filter(|name| terminal_available || !notia_backend_core::terminal::is_terminal_tool(name))
        .filter(|name| browser_available || !notia_backend_core::browser::is_browser_tool(name))
        .filter(|name| whatsapp_connected || !notia_backend_core::whatsapp::is_whatsapp_tool(name))
        .filter(|name| crate::agent_scripts::is_available(app) || !notia_backend_core::agent_scripts::is_script_tool(name))
        .filter(|name| runs_services || !crate::ai_task_runner::is_ai_task_write_tool(name))
        .collect::<Vec<_>>();
    let requested_tool_names = if requested_tool_names.is_empty() {
        available_tool_names
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    } else {
        requested_tool_names
            .into_iter()
            .filter(|name| available_tool_names.iter().any(|available| *available == name))
            .collect::<Vec<_>>()
    };
    state
        .journal
        .hydrate_record(app, registry, &request.context, &request.idempotency_key)?;
    // The chat's switches narrow what the actor and the scope allow.
    request.tools = notia_backend_core::restrict_tool_access(
        project_canonical_tool_catalog(
            &request.context,
            &principal,
            &requested_tool_names,
            ToolCatalogProjection::Full,
        )?,
        request.tool_access,
        request.library_search,
    );
    // An AI action or a webhook call runs for the Owner, in their Telegram
    // chat, with their memory, rules and thoughts; nothing else may claim
    // to be one.
    if (request.scheduled_action.is_some() || request.webhook_call.is_some())
        && !(request.context.actor.is_library_owner()
            && request.context.persistence_policy.allows_memory()
            && request.context.channel == notia_backend_core::BackendChannel::Telegram)
    {
        return Err(BackendError::new(
            BackendErrorCode::Forbidden,
            "Las acciones de IA y los webhooks solo corren para el Owner, por Telegram.",
            false,
        ));
    }
    // A run Notia started by itself only reads and keeps its thoughts.
    if request.autonomous {
        request.tools = notia_backend_core::agent_autonomy::autonomous_tools(std::mem::take(&mut request.tools));
    }
    // A phase of an AI task runs for the Owner with memory, with its
    // phase's tools; nothing else may claim to be one.
    if let Some(turn) = &request.ai_task {
        if !(request.context.actor.is_library_owner() && request.context.persistence_policy.allows_memory()) {
            return Err(BackendError::new(BackendErrorCode::Forbidden, "Las Tareas IA solo corren para el Owner.", false));
        }
        request.tools = notia_backend_core::ai_tasks::tools::phase_tools(std::mem::take(&mut request.tools), turn.phase, request.library_search);
    } else {
        request.tools = notia_backend_core::ai_tasks::tools::without_task_wait(std::mem::take(&mut request.tools));
    }
    // `thoughts.md`, `biography.md`, `talk.md` and `people.md` are always
    // present for the runs that read them, even when deleted during the
    // session; a new `people.md` starts with the people the memory names.
    if request.context.persistence_policy.allows_memory() && request.context.actor.is_library_owner() {
        match crate::agent_workspace::ensure_agent_list_files(app, &request.context.library_id) {
            Ok(written) if written.contains(&notia_backend_core::agent_workspace::PEOPLE_PATH) => {
                crate::agent_knowledge::schedule_organization(app, &request.context.library_id)
            }
            Ok(_) => {}
            Err(error) => log::error!("[notia:memory] no se pudieron recrear los archivos del agente: {:?}", error.code),
        }
    }
    // Every chat with more than one area is routed: the turn keeps every
    // tool the actor may use and offers those of its areas, which the agent
    // may change mid-run, and the same call decides whether the turn thinks
    // and, on Telegram, whether it answers with the conversational prompt
    // (see `backend_core::tool_routing`). A resumed run keeps all three.
    if fresh_run {
        let routed = route_turn_areas(app, state, &request);
        request.thinking = routed.as_ref().and_then(|routed| routed.thinking);
        request.conversational = routed.as_ref().is_some_and(|routed| routed.conversational);
        request.tool_areas = routed.map(|routed| routed.areas);
        // The support scripts reach the model in the description of the tool
        // that runs them, like tools of their own.
        crate::agent_scripts::describe_scripts(app, &request.context.library_id, &mut request.tools);
    }
    state.journal.store_request(&request)?;
    let provider_settings = provider_settings_for_library(app, state, &request.context.library_id)?;
    // The same model checks the replies that only announce a step, so the
    // agent keeps working instead of stopping on them.
    let continuation_judge = notia_backend_core::continuation::ContinuationJudge(Arc::new(quick_provider(app, &provider_settings)?));
    let provider = OllamaAgentProvider::with_transport(
        OllamaProviderConfig::new(
            crate::services::ai_service::AiHttpSettings {
                ollama_url: provider_settings.ollama_url,
                api_key: provider_settings.api_key,
            },
            provider_settings.model,
            // A conversational turn answers without thinking.
            if request.thinking == Some(false) { Value::Bool(false) } else { provider_settings.think },
        )?,
        platform_ollama_transport(app),
    );
    let mut options = AgentRuntimeOptions::default();
    options.continuation_judge = Some(continuation_judge);
    options.projection = if matches!(request.context.persistence_policy, PersistencePolicy::PublishedNoMemory)
    {
        ToolCatalogProjection::PublishedTaskManager
    } else if matches!(request.context.scope, BackendScope::Graph) {
        ToolCatalogProjection::ReadOnly
    } else {
        ToolCatalogProjection::Full
    };
    let provider = AuthoritativePromptProvider {
        inner: &provider,
        compose: Box::new(|tools| compose_request_system_prompt(app, &request, tools)),
        composed: Mutex::new(None),
    };
    // The run works until the task is done: no overall deadline. Each model
    // call and each tool keeps its own timeout, and the person can cancel.
    let control = RequestControl::new(None);
    let sink = TauriBackendEventSink::for_request(app.clone(), Arc::clone(&state.events), &request.context);
    let executor = TauriBackendToolExecutor {
        app: app.clone(),
        snapshot: request.snapshot.clone(),
        web_search_tracker: Mutex::new(WebSearchTracker::default()),
        control: control.clone(),
        journal: Arc::clone(&state.journal),
        idempotency_key: request.idempotency_key.clone(),
        request_images: last_user_images(&request),
        attachment_images: conversation_images(&request),
        events: Some(sink.clone()),
        live_status: request.ai_task.is_none(),
    };
    let revisions = TauriRevisionPort { app: app.clone() };
    let interactions = state.interactions(Some(&revisions));
    let result = if let Some((operation_id, decision)) = resume_decision {
        interactions.register_control(&request.context, control.clone())?;
        interactions
            .run_agent_with_resume(
                &provider,
                &executor,
                state.journal.as_ref(),
                &sink,
                &request,
                &principal,
                &control,
                &options,
                &operation_id,
                decision,
            )
            .map(|result| match result {
                notia_backend_core::AgentRunResult::Completed(response) => {
                    BackendResponse::Result { response }
                }
                notia_backend_core::AgentRunResult::Waiting(status) => {
                    BackendResponse::Operation { status }
                }
            })
    } else {
        interactions.run_agent_request(
            &provider,
            &executor,
            state.journal.as_ref(),
            &sink,
            &request,
            &principal,
            &control,
            &options,
        )
    };
    // A waiting run keeps no active control: a later cancel targets the
    // stored interaction instead of a worker that already returned.
    let _ = interactions.unregister_control(&request.context);
    state
        .journal
        .persist_record(app, registry, &request.context, &request.idempotency_key)?;
    // A request that ended, however it ended, is reflected on; a paused one
    // waits for its end.
    let ending = match &result {
        Ok(BackendResponse::Operation { .. }) => None,
        Ok(BackendResponse::Error { error, .. }) => Some(Some(format!("No terminó: {}", error.message))),
        Ok(_) => Some(None),
        Err(error) if error.code == BackendErrorCode::Conflict => None,
        Err(error) if error.code == BackendErrorCode::Cancelled => Some(Some(CANCELLED_TURN_ENDING.to_string())),
        Err(error) => Some(Some(format!("No terminó: {}", error.message))),
    };
    if let Some(ending) = ending {
        schedule_turn_reflection(app, state, &request.context, &request.idempotency_key, ending);
    }
    let response = result?;
    Ok(BackendResponseEnvelope {
        protocol_version,
        response,
    })
}

/// Provider settings from the `ia` section of a library configuration.
fn provider_settings_from_config(config: &Value) -> Result<BackendProviderSettings, BackendError> {
    let ia = config.get("ia").cloned().unwrap_or(Value::Null);
    let text = |key: &str| ia.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let (ollama_url, model) = (text("ollamaUrl"), text("selectedModel"));
    if ollama_url.is_empty() || model.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::ProviderUnavailable,
            "Configurá la URL de Ollama y el modelo en Configuración.",
            false,
        ));
    }
    let think = if ia.get("thinkingEnabled").and_then(Value::as_bool) != Some(false) {
        Value::String(ia.get("thinkingLevel").and_then(Value::as_str).unwrap_or("medium").to_string())
    } else {
        Value::Bool(false)
    };
    Ok(BackendProviderSettings { ollama_url, model, api_key: text("apiKey"), think })
}

/// Provider of one library: its saved AI settings (URL, model, credential and
/// thinking), so a run never uses the credential of another library. The
/// runtime-wide settings are only a fallback for a library without them.
/// The library's Ollama endpoint when it runs on this computer (and so on
/// its GPU); `None` for ollama.com and other hosts.
pub(crate) fn local_ollama_url(app: &AppHandle, library_id: &str) -> Option<String> {
    let state = app.state::<BackendRuntimeState>();
    let url = provider_settings_for_library(app, state.inner(), library_id).ok()?.ollama_url;
    let host = url.split("://").nth(1)?.split(['/', ':']).next()?.to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1").then_some(url)
}

fn provider_settings_for_library(
    app: &AppHandle,
    state: &BackendRuntimeState,
    library_id: &str,
) -> Result<BackendProviderSettings, BackendError> {
    let config = crate::library_config::read_library_config(app, library_id)?.unwrap_or(Value::Null);
    if let Ok(settings) = provider_settings_from_config(&config) {
        return Ok(settings);
    }
    state
        .provider
        .lock()
        .map_err(|_| internal_error("No se pudo leer la configuración del proveedor backend."))?
        .clone()
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ProviderUnavailable,
                "Configurá la URL de Ollama y el modelo en Configuración.",
                false,
            )
        })
}

/// Longest wait for the routing call; a slower model leaves the choice to
/// words. It covers loading a local model, which the turn then reuses.
const ROUTING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Tool areas of a turn: the model reads the recent conversation and picks
/// the areas it needs among those the actor may use (the tools come already
/// projected for the actor's contexts); a module's chat suggests its own
/// area, which also decides with words when the model does not answer.
/// The same answer says whether the turn needs thinking (`None` when the
/// model does not say) and, in a Telegram turn the person wrote, whether it
/// answers with the conversational prompt. `None` when the tools span a
/// single area. See `backend_core::tool_routing`.
fn route_turn_areas(app: &AppHandle, state: &BackendRuntimeState, request: &AgentRequest) -> Option<RoutedTurn> {
    use notia_backend_core::tool_routing::{
        area_ids, fallback_areas, home_area, needs_routing, offered_areas, parse_router_answer, router_prompt, with_prompt_choice,
        RouterMessage, MAX_ROUTER_IMAGES,
    };
    use notia_backend_core::{BackendMessage, MessageRole};
    // The published board only reaches its tickets.
    if !needs_routing(&request.tools) || matches!(request.context.persistence_policy, PersistencePolicy::PublishedNoMemory) {
        return None;
    }
    let offered = offered_areas(&request.tools);
    let home = home_area(&request.context.scope, &request.context.channel);
    let attachment_count = |message: &BackendMessage| message.images.len() + message.attachments.len();
    // The last request goes with the text of its files and its first images:
    // the attachment itself tells what kind of request it is.
    let last_index = request.messages.iter().rposition(|message| message.role == MessageRole::User);
    let (last_text, last_images) = last_index.map_or((String::new(), Vec::new()), |index| {
        let message = &request.messages[index];
        let (text, pages) = notia_backend_core::chat_attachments::compose_message(&message.content, &message.attachments);
        (text, message.images.iter().cloned().chain(pages).take(MAX_ROUTER_IMAGES).collect::<Vec<_>>())
    });
    let with_attachments = last_index.is_some_and(|index| attachment_count(&request.messages[index]) > 0);
    let conversation = request
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| matches!(message.role, MessageRole::User | MessageRole::Assistant))
        .map(|(index, message)| RouterMessage {
            from_user: message.role == MessageRole::User,
            content: if Some(index) == last_index { last_text.as_str() } else { message.content.as_str() },
            attachments: attachment_count(message),
        })
        .collect::<Vec<_>>();
    let (system, user) = router_prompt(&offered, &conversation, home);
    // Only a message the person wrote on Telegram may get the conversational prompt.
    let offers_conversation = request.context.channel == notia_backend_core::BackendChannel::Telegram
        && !request.autonomous
        && request.scheduled_action.is_none()
        && request.webhook_call.is_none();
    let system = if offers_conversation { with_prompt_choice(system) } else { system };
    let decision = provider_settings_for_library(app, state, &request.context.library_id)
        .and_then(|settings| complete_with(app, settings, request.context.clone(), &system, &user, last_images, ROUTING_TIMEOUT))
        .map(|answer| parse_router_answer(&answer, &offered))
        .unwrap_or_default();
    let (areas, source) = match decision.areas {
        Some(areas) => (areas, "modelo"),
        None => (fallback_areas(&last_text, &offered, with_attachments, home), "palabras"),
    };
    let conversational = offers_conversation && decision.conversational == Some(true);
    log::info!(
        "[notia:router] áreas {:?} elegidas por {source}; thinking {:?}; prompt {}",
        areas.iter().map(|area| area.id()).collect::<Vec<_>>(),
        decision.thinking,
        if conversational { "conversacional" } else { "default" }
    );
    Some(RoutedTurn { areas: area_ids(&areas), thinking: decision.thinking, conversational })
}

/// What the routing call decided for a fresh run.
struct RoutedTurn {
    areas: Vec<String>,
    thinking: Option<bool>,
    conversational: bool,
}

/// One completion without tools for a background task of the library
/// (titles, memories), with the library's saved AI settings, waiting at
/// most `timeout`.
pub(crate) fn complete_text(
    app: &AppHandle,
    library_id: &str,
    system: &str,
    user: &str,
    timeout: std::time::Duration,
) -> Result<String, BackendError> {
    let config = crate::library_config::read_library_config(app, library_id)?.unwrap_or(Value::Null);
    let settings = provider_settings_from_config(&config)?;
    let context = BackendRequestContext {
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        library_id: library_id.to_string(),
        actor: notia_backend_core::BackendActor { library_user_id: "user-owner".to_string(), external_identity: None },
        channel: notia_backend_core::BackendChannel::App,
        scope: BackendScope::Library,
        persistence_policy: PersistencePolicy::EphemeralNoMemory,
    };
    complete_with(app, settings, context, system, user, Vec::new(), timeout)
}

/// Like [`complete_text`], with images (base64) for the model to look at.
pub(crate) fn complete_with_images(
    app: &AppHandle,
    library_id: &str,
    system: &str,
    user: &str,
    images: Vec<String>,
    timeout: std::time::Duration,
) -> Result<String, BackendError> {
    let config = crate::library_config::read_library_config(app, library_id)?.unwrap_or(Value::Null);
    let settings = provider_settings_from_config(&config)?;
    let context = BackendRequestContext {
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        library_id: library_id.to_string(),
        actor: notia_backend_core::BackendActor { library_user_id: "user-owner".to_string(), external_identity: None },
        channel: notia_backend_core::BackendChannel::App,
        scope: BackendScope::Library,
        persistence_policy: PersistencePolicy::EphemeralNoMemory,
    };
    complete_with(app, settings, context, system, user, images, timeout)
}

/// Decides what to do with a message sent while a request of the same chat
/// runs: stop it, stop it and run the message next, or queue the message.
/// A short call to the library's model decides, in parallel with the run;
/// an explicit command needs no model and words decide when it does not
/// answer in time (see `backend_core::turn_interrupts`).
pub(crate) fn classify_interrupt(
    app: &AppHandle,
    context: &BackendRequestContext,
    running: &str,
    message: &str,
) -> notia_backend_core::turn_interrupts::InterruptDecision {
    use notia_backend_core::turn_interrupts::{
        fallback_decision, interrupt_prompt, is_cancel_command, parse_interrupt_answer, InterruptDecision,
        INTERRUPT_TIMEOUT,
    };
    if is_cancel_command(message) {
        return InterruptDecision::Cancel;
    }
    let state = app.state::<BackendRuntimeState>();
    let (system, user) = interrupt_prompt(running, message);
    let context = BackendRequestContext {
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        persistence_policy: PersistencePolicy::EphemeralNoMemory,
        ..context.clone()
    };
    let decided = provider_settings_for_library(app, state.inner(), &context.library_id)
        .and_then(|settings| complete_with(app, settings, context, &system, &user, Vec::new(), INTERRUPT_TIMEOUT))
        .ok()
        .and_then(|answer| parse_interrupt_answer(&answer));
    let (decision, source) = match decided {
        Some(decision) => (decision, "modelo"),
        None => (fallback_decision(message), "palabras"),
    };
    log::info!("[notia:interrupt] {decision:?} decidido por {source}");
    decision
}

/// Provider for short calls without tools or thinking: titles, memories and
/// the checks that run beside the agent.
fn quick_provider(app: &AppHandle, settings: &BackendProviderSettings) -> Result<OllamaAgentProvider, BackendError> {
    Ok(OllamaAgentProvider::with_transport(
        OllamaProviderConfig::new(
            crate::services::ai_service::AiHttpSettings {
                ollama_url: settings.ollama_url.clone(),
                api_key: settings.api_key.clone(),
            },
            settings.model.clone(),
            Value::Bool(false),
        )?,
        platform_ollama_transport(app),
    ))
}

/// One completion without tools, without thinking, within `timeout`; the
/// `images` go with the user message.
#[allow(clippy::too_many_arguments)]
fn complete_with(
    app: &AppHandle,
    settings: BackendProviderSettings,
    context: BackendRequestContext,
    system: &str,
    user: &str,
    images: Vec<String>,
    timeout: std::time::Duration,
) -> Result<String, BackendError> {
    let provider = quick_provider(app, &settings)?;
    let message = |role, content: &str| notia_backend_core::ProviderMessage {
        role,
        content: content.to_string(),
        images: Vec::new(),
        tool_calls: Vec::new(),
        tool_name: None,
    };
    let request = notia_backend_core::ProviderRequest {
        context,
        messages: vec![
            message(notia_backend_core::ProviderMessageRole::System, system),
            notia_backend_core::ProviderMessage { images, ..message(notia_backend_core::ProviderMessageRole::User, user) },
        ],
        tools: Vec::new(),
    };
    let control = RequestControl::new(Some(timeout));
    use notia_backend_core::AgentProvider as _;
    let content = provider.chat(&request, &control)?.message.content;
    // Background tasks (titles, memories, routing) need an answer; an empty
    // one is an error for them, never an empty title or memory.
    if content.trim().is_empty() {
        return Err(BackendError::new(BackendErrorCode::ProviderUnavailable, "La IA no devolvió contenido.", true));
    }
    Ok(content)
}

fn internal_error(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Internal, message, true)
}

/// Logs why an agent request failed, for diagnosis: its error code, the
/// message only for system errors (a data or validation message may quote
/// private content) and how many times it ran each tool.
pub(crate) fn log_request_failure(runtime: &BackendRuntimeState, channel: &str, context: &BackendRequestContext, error: &BackendError) {
    let system_error = matches!(
        error.code,
        BackendErrorCode::ProviderUnavailable | BackendErrorCode::Timeout | BackendErrorCode::Internal
    );
    let events = runtime.request_events(context, 0).unwrap_or_default();
    // Error level: the app's logger only shows errors.
    log::error!(
        "[notia:{channel}] la solicitud {} falló ({:?}){}; herramientas: {}",
        context.request_id,
        error.code,
        if system_error { format!(": {}", error.message) } else { String::new() },
        tool_usage(events.iter().map(|envelope| &envelope.event)),
    );
}

/// «search_gmail_messages 3, trash_gmail_messages 2 (1 falló)», or «ninguna».
fn tool_usage<'a>(events: impl Iterator<Item = &'a notia_backend_core::BackendEvent>) -> String {
    let mut tools = std::collections::BTreeMap::<&str, (u32, u32)>::new();
    for event in events {
        if let notia_backend_core::BackendEvent::ToolCompleted { tool_name, ok, .. } = event {
            let counts = tools.entry(tool_name.as_str()).or_default();
            if *ok {
                counts.0 += 1;
            } else {
                counts.1 += 1;
            }
        }
    }
    if tools.is_empty() {
        return "ninguna".to_string();
    }
    tools
        .iter()
        .map(|(name, (ok, failed))| match failed {
            0 => format!("{name} {ok}"),
            1 => format!("{name} {} (1 falló)", ok + failed),
            _ => format!("{name} {} ({failed} fallaron)", ok + failed),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// User-facing name of a serde enum value (e.g. `Pendiente`).
fn serde_label(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Confirmation text of a Task Manager tool. `groups` and `comments` are the
/// library's current ones: the text names a group and quotes a comment
/// whole, never by its id.
fn task_mutation_summary(
    mutation: &notia_backend_core::TaskMutationDto,
    groups: &[notia_backend_core::TaskGroupDto],
    comments: &[notia_backend_core::TaskCommentDto],
) -> String {
    use notia_backend_core::TaskMutationDto as Mutation;
    let group_name = |group_id: &str| {
        groups
            .iter()
            .find(|group| group.group_id == group_id)
            .map_or_else(|| group_id.to_string(), |group| format!("«{}»", group.name))
    };
    let comment_text = |comment_id: &str| {
        comments
            .iter()
            .find(|comment| comment.comment_id == comment_id)
            .map_or_else(|| comment_id.to_string(), |comment| format!("«{}»", comment.body.trim()))
    };
    match mutation {
        Mutation::CreateTicket { title, .. } => format!("Crear el ticket «{title}»."),
        Mutation::ReplaceTicketContent { ticket_id, .. } => {
            format!("Reemplazar el contenido del ticket {ticket_id}.")
        }
        Mutation::AddComment { ticket_id, body, .. } => {
            format!("Comentar en el ticket {ticket_id}: «{body}».")
        }
        Mutation::UpdateComment { ticket_id, comment_id, body } => format!(
            "Cambiar el comentario {} del ticket {ticket_id} por «{body}».",
            comment_text(comment_id)
        ),
        Mutation::DeleteComment { ticket_id, comment_id } => {
            format!("Eliminar el comentario {} del ticket {ticket_id}.", comment_text(comment_id))
        }
        Mutation::AddSubtask {
            parent_ticket_id,
            title,
            ..
        } => format!("Crear la subtarea «{title}» en el ticket {parent_ticket_id}."),
        Mutation::MoveTicket { ticket_id, group_id } => match group_id {
            Some(group_id) => format!("Mover el ticket {ticket_id} al grupo {}.", group_name(group_id)),
            None => format!("Quitar el grupo del ticket {ticket_id}."),
        },
        Mutation::ChangeState { ticket_id, state } => {
            format!("Cambiar el estado del ticket {ticket_id} a {}.", serde_label(state))
        }
        Mutation::ChangePriority {
            ticket_id,
            priority,
        } => format!("Cambiar la prioridad del ticket {ticket_id} a {}.", serde_label(priority)),
        Mutation::UpdateTicket { ticket_id, .. } => format!("Actualizar campos del ticket {ticket_id}."),
        Mutation::BulkUpdate { ticket_ids, .. } => {
            format!("Actualizar {} tickets con los mismos campos.", ticket_ids.len())
        }
        Mutation::DuplicateTicket { ticket_id, .. } => format!("Duplicar el ticket {ticket_id}."),
        Mutation::ArchiveTicket { ticket_id } => format!("Archivar el ticket {ticket_id}."),
        Mutation::RestoreTicket { ticket_id } => format!("Restaurar el ticket {ticket_id}."),
        Mutation::CreateGroup { name, color, .. } => {
            format!("Crear el grupo «{name}» con color {color}.")
        }
        Mutation::UpdateGroup { group_id, name, color, .. } => {
            let current = groups.iter().find(|group| group.group_id == *group_id);
            let mut changes = Vec::new();
            if current.is_none_or(|group| group.name != *name) {
                changes.push(format!("nombre «{name}»"));
            }
            if current.is_none_or(|group| group.color != *color) {
                changes.push(format!("color {color}"));
            }
            if changes.is_empty() {
                format!("Guardar el grupo {} sin cambios.", group_name(group_id))
            } else {
                format!("Cambiar el grupo {}: {}.", group_name(group_id), changes.join(" y "))
            }
        }
        Mutation::ReorderGroups { group_ids, .. } => format!(
            "Ordenar los grupos: {}.",
            group_ids.iter().map(|group_id| group_name(group_id)).collect::<Vec<_>>().join(" → ")
        ),
        Mutation::DeleteGroup { group_id, .. } => format!("Eliminar el grupo {}.", group_name(group_id)),
        other => format!("Aplicar la mutación de Task Manager {other:?}."),
    }
}

/// Images of the last message of the person: its images and the pages of
/// its image attachments, in order.
fn last_user_images(request: &AgentRequest) -> Vec<crate::recipes::MessageImage> {
    let Some(message) = request.messages.iter().rev().find(|message| message.role == notia_backend_core::MessageRole::User) else {
        return Vec::new();
    };
    let attached = message
        .attachments
        .iter()
        .filter(|attachment| attachment.kind == notia_backend_core::chat_attachments::MessageAttachmentKind::Image)
        .flat_map(|attachment| attachment.pages.iter().take(1));
    message.images.iter().chain(attached).map(|base64| crate::recipes::MessageImage { base64: base64.clone() }).collect()
}

/// Image attachments of the conversation, by the reference the model was
/// told to insert them with: the person's photos and the images the agent
/// made in earlier replies. Earlier messages count too: a photo sent before
/// can still go into a note or be edited when asked later, and it is
/// inserted only where the model writes its reference. A later reference
/// wins a repeated one.
fn conversation_images(request: &AgentRequest) -> Vec<(String, String)> {
    let mut images = Vec::<(String, String)>::new();
    for message in request.messages.iter().filter(|message| message.role != notia_backend_core::MessageRole::System) {
        let references = notia_backend_core::chat_attachments::image_references(&message.attachments);
        for (reference, attachment) in references.into_iter().zip(&message.attachments) {
            let (Some(reference), Some(page)) = (reference, attachment.pages.first()) else {
                continue;
            };
            images.retain(|(known, _)| *known != reference);
            images.push((reference, page.clone()));
        }
    }
    images
}

/// Current UTC date as `YYYY-MM-DD`.
fn utc_today() -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    crate::task_manager_store::format_iso_timestamp(now_ms)[..10].to_string()
}

fn storage_error(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, message, true)
}

fn invalid_result() -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        "El backend no pudo serializar el resultado de la tool.",
        true,
    )
}

/// The reason serde gives (an unknown type, a missing field) tells the model
/// what to fix, instead of a bare refusal it can only repeat.
fn invalid_markdown_operation(error: serde_json::Error) -> BackendError {
    BackendError::invalid_input(format!(
        "La operación Markdown no es válida ({error}). Usá {{type, payload}} con un tipo de la lista; para cambiar una frase del cuerpo, replaceText {{find, replace}} con find copiado tal cual del documento."
    ))
}

/// `expectedRevision` of a tool call or of one of its edits. Some models quote
/// the revision; both forms are the same number.
fn revision_argument(arguments: &Value) -> Option<u64> {
    arguments.get("expectedRevision").and_then(|value| {
        value.as_u64().or_else(|| value.as_str().and_then(|text| text.trim().parse::<u64>().ok()))
    })
}

/// What a Markdown preview tells the model: the document, its revision and
/// the changes, not the whole text. The apply takes the same path and
/// operation and computes the preview again.
fn compact_markdown_preview(preview: &MarkdownEditPreview) -> Value {
    json!({
        "path": preview.locator.logical_path.as_str(),
        "expectedRevision": preview.expected_revision,
        "hunks": preview
            .hunks
            .iter()
            .map(|hunk| json!({
                "id": hunk.id,
                "startLine": hunk.anchor.start_line,
                "endLine": hunk.anchor.end_line,
                "oldText": hunk.old_text,
                "newText": hunk.new_text,
            }))
            .collect::<Vec<_>>(),
    })
}

fn unsupported(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, message, false)
}

/// Learns from a request of the Owner that ended (answered, failed or
/// cancelled): a background call reads what happened and saves the new
/// memories and thoughts, so a long run keeps what it learned even when the
/// model never saved it. Runs Notia started by itself keep their own
/// thoughts and are left out.
fn schedule_turn_reflection(
    app: &AppHandle,
    state: &BackendRuntimeState,
    context: &BackendRequestContext,
    idempotency_key: &str,
    ending: Option<String>,
) {
    if !context.persistence_policy.allows_memory() || !context.actor.is_library_owner() {
        return;
    }
    let Some((request, tools, response)) = state.journal.turn_record(context, idempotency_key) else {
        return;
    };
    // The phases of an AI task keep their work in the task itself.
    if request.autonomous || request.ai_task.is_some() {
        return;
    }
    let asked = request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == notia_backend_core::MessageRole::User)
        .map(|message| message.content.clone())
        .unwrap_or_default();
    let notes = state
        .request_events(context, 0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            notia_backend_core::BackendEvent::AssistantNote { text, .. } if !text.trim().is_empty() => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    let tools = tools
        .into_iter()
        .map(|(key, result)| {
            let (name, arguments) = key.split_once(':').unwrap_or((key.as_str(), ""));
            notia_backend_core::agent_knowledge::ToolDigest {
                name: name.to_string(),
                arguments: arguments.to_string(),
                ok: result.ok,
                result: match (&result.data, &result.error) {
                    (_, Some(error)) => error.message.clone(),
                    (Some(data), None) => data.to_string(),
                    (None, None) => String::new(),
                },
            }
        })
        .collect::<Vec<_>>();
    let ending = ending
        .or_else(|| response.map(|response| response.response.markdown))
        .unwrap_or_else(|| "El turno terminó sin respuesta final.".to_string());
    let Some(transcript) = notia_backend_core::agent_knowledge::reflection_transcript(&asked, &notes, &tools, &ending) else {
        return;
    };
    crate::agent_knowledge::schedule_reflection(app, &context.library_id, idempotency_key, transcript);
}

fn request_identity(request: &BackendRequest) -> (&BackendRequestContext, &str) {
    match request {
        BackendRequest::Run(request) => (&request.context, &request.idempotency_key),
        BackendRequest::Resume(request) => (&request.context, &request.idempotency_key),
        BackendRequest::Cancel(request) => (&request.context, &request.idempotency_key),
        BackendRequest::GetOperation(request) => (&request.context, &request.idempotency_key),
        BackendRequest::Review(request) => (&request.context, &request.idempotency_key),
        BackendRequest::Undo(request) => (&request.context, &request.idempotency_key),
    }
}

#[cfg(test)]
mod tests {
    use super::TauriBackendToolExecutor;

    #[test]
    fn document_tools_report_the_notes_they_wrote() {
        use serde_json::json;
        assert_eq!(super::changed_document_paths(Some(&json!({ "path": "Ideas/a.md", "changed": true }))), vec!["Ideas/a.md"]);
        assert_eq!(
            super::changed_document_paths(Some(&json!({ "documents": [{ "path": "a.md" }, { "path": "b/c.md" }] }))),
            vec!["a.md", "b/c.md"]
        );
        assert!(super::changed_document_paths(Some(&json!({ "changed": false }))).is_empty());
        assert!(super::changed_document_paths(None).is_empty());
        assert!(super::is_document_write_tool("apply_markdown_edit"));
        assert!(!super::is_document_write_tool("save_finance_account"));
    }

    /// Telegram, 2026-10-02: a lunch photo sent at 17:00 rode along with
    /// every later request for hours, so the agent kept offering to load it
    /// (and tried to). A photo of an earlier message is not the request's
    /// photo for the tools, but it can still go into a note by reference.
    #[test]
    fn a_photo_of_an_earlier_message_is_not_the_requests_photo() {
        use notia_backend_core::chat_attachments::{MessageAttachment, MessageAttachmentKind};
        use notia_backend_core::{
            AgentRequest, BackendActor, BackendChannel, BackendMessage, BackendRequestContext, BackendScope, MessageRole,
            PersistencePolicy,
        };
        let photo = MessageAttachment {
            name: "almuerzo.jpg".into(),
            media_type: "image/jpeg".into(),
            kind: MessageAttachmentKind::Image,
            pages: vec!["QUJD".into()],
            text_content: None,
            extracted_text: None,
            page_count: None,
        };
        let said = |role, content: &str, attachments| BackendMessage { role, content: content.into(), images: Vec::new(), attachments };
        let drawn = MessageAttachment { name: "imagen-2026-10-13-01-30-00.png".into(), media_type: "image/png".into(), pages: vec!["UE5H".into()], ..photo.clone() };
        let history = vec![
            said(MessageRole::User, "Cargame este almuerzo", vec![photo]),
            said(MessageRole::Assistant, "Quedó registrado.", Vec::new()),
            said(MessageRole::User, "Dibujame el plato", Vec::new()),
            said(MessageRole::Assistant, "Acá está.", vec![drawn]),
        ];
        let request = |text: &str| AgentRequest {
            context: BackendRequestContext {
                request_id: "request-1".into(),
                library_id: "library-1".into(),
                actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
                channel: BackendChannel::Telegram,
                scope: BackendScope::Library,
                persistence_policy: PersistencePolicy::Persistent,
            },
            messages: notia_backend_core::telegram_bot::turn_messages(&history, text, Vec::new()),
            snapshot: None,
            tools: Vec::new(),
            attachments: Vec::new(),
            idempotency_key: "key-1".into(),
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
        };
        let later = request("¿Me pasás la rutina de hoy?");
        assert!(super::last_user_images(&later).is_empty());
        // The images the agent drew can be edited or put in a note too.
        assert_eq!(
            super::conversation_images(&later),
            vec![("almuerzo.jpg".to_string(), "QUJD".to_string()), ("imagen-2026-10-13-01-30-00.png".to_string(), "UE5H".to_string())]
        );
    }

    /// «Haceme una nota que es de una idea…» from Telegram (2026-09-29): the
    /// agent's note, previewed, confirmed and written, in the library root,
    /// in an existing folder and in a new one, with a long title and quotes.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn the_agent_saves_a_note_it_is_asked_for() {
        use crate::host::{AppContext, AppPaths, HostPorts, Manager};
        use notia_backend_core::{
            BackendActor, BackendChannel, BackendRequestContext, BackendScope, ConfirmationDecision,
            PersistencePolicy, RequestControl, ToolCall, ToolExecutor,
        };
        use std::sync::{Arc, Mutex};

        let root = std::env::temp_dir().join(format!("notia-agent-note-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Personal")).expect("root");
        let app = AppContext::new(AppPaths::default(), HostPorts::default());
        let registry = crate::library_registry::LibraryBindingRegistry::default();
        registry.register_desktop_root("library-1", &root).expect("binding");
        app.manage(registry);
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        app.manage(crate::agent_history::AgentHistoryState::default());
        // A photo of the person's message, as Telegram names it.
        let mut png = Vec::new();
        image::RgbImage::from_pixel(4, 4, image::Rgb([200, 120, 40]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        let photo = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(png)
        };
        let executor = TauriBackendToolExecutor {
            app: app.clone(),
            snapshot: None,
            web_search_tracker: Mutex::new(Default::default()),
            control: RequestControl::new(None),
            journal: Arc::new(Default::default()),
            idempotency_key: "key-1".into(),
            events: None,
            live_status: false,
            request_images: Vec::new(),
            attachment_images: vec![("telegram-AgAC_1.jpg".into(), photo)],
        };
        let context = BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
            channel: BackendChannel::App,
            scope: BackendScope::Library,
            persistence_policy: PersistencePolicy::Persistent,
        };
        let idea = "Idea: retirar el chárter de los productos moviendo la evaluación al límite de la consulta, que se \"lockee\" mientras la consulta no esté confirmada";
        for (index, folder) in ["", "Personal/", "Ideas/"].into_iter().enumerate() {
            let call = ToolCall {
                id: format!("call-{index}"),
                name: "create_library_note".into(),
                arguments: serde_json::json!({ "path": format!("{folder}{idea}.md"), "content": format!("# Idea\n\n{idea}\n") }),
                round: 1,
            };
            let preview = executor.preview(&context, &call).expect("preview").expect("a preview");
            let decision = ConfirmationDecision {
                operation_id: preview.operation_id.clone(),
                accepted: true,
                hunk_ids: Vec::new(),
                approve_all: false,
                suggestion: None,
            };
            let result = executor.execute_confirmed(&context, &call, &decision, Some(&preview)).expect("created");
            assert!(result.ok, "{folder}: {:?}", result.error);
            let path = result.data.as_ref().and_then(|data| data["path"].as_str()).expect("path").to_string();
            assert_eq!(preview.documents[0].path, path);
            assert!(std::fs::read_to_string(root.join(&path)).expect("note").contains("lockee"), "{path}");
        }
        // A note that inserts the photo keeps it inside the file; the
        // preview keeps the short reference.
        let call = ToolCall {
            id: "call-photo".into(),
            name: "create_library_note".into(),
            arguments: serde_json::json!({ "path": "Salud/Progreso.md", "content": "# Progreso\n\n![Frente](telegram-AgAC_1.jpg)\n" }),
            round: 1,
        };
        let preview = executor.preview(&context, &call).expect("preview").expect("a preview");
        let decision = ConfirmationDecision {
            operation_id: preview.operation_id.clone(),
            accepted: true,
            hunk_ids: Vec::new(),
            approve_all: false,
            suggestion: None,
        };
        let result = executor.execute_confirmed(&context, &call, &decision, Some(&preview)).expect("created");
        assert!(result.ok, "{:?}", result.error);
        let note = std::fs::read_to_string(root.join("Salud/Progreso.md")).expect("note");
        assert!(note.contains("![Frente](data:image/jpeg;base64,"), "{note}");
        assert!(!note.contains("telegram-AgAC_1.jpg"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// The Telegram request of 2026-10-14: fix one sentence of a meeting note.
    /// Ollama reads tool arguments as `float64`, so every number the model
    /// sends back goes through one.
    #[test]
    fn the_agent_fixes_a_sentence_of_a_note_from_telegram() {
        use crate::host::{AppContext, AppPaths, HostPorts, Manager};
        use notia_backend_core::{
            BackendActor, BackendChannel, BackendRequestContext, BackendScope, PersistencePolicy, RequestControl,
            ToolCall, ToolExecutor,
        };
        use serde_json::{json, Value};
        use std::sync::{Arc, Mutex};

        fn through_float(value: &Value) -> Value {
            match value {
                Value::Number(number) => serde_json::from_str(&format!("{}", number.as_f64().expect("number"))).expect("number"),
                Value::Array(items) => Value::Array(items.iter().map(through_float).collect()),
                Value::Object(fields) => Value::Object(fields.iter().map(|(key, item)| (key.clone(), through_float(item))).collect()),
                other => other.clone(),
            }
        }

        let root = std::env::temp_dir().join(format!("notia-agent-sentence-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("root");
        let source = "# Meet\n\n- La extensión del horario de remesas de 0 a 3.\n- Otro punto.\n";
        std::fs::write(root.join("Meet.md"), source).expect("note");
        let app = AppContext::new(AppPaths::default(), HostPorts::default());
        let registry = crate::library_registry::LibraryBindingRegistry::default();
        registry.register_desktop_root("library-1", &root).expect("binding");
        app.manage(registry);
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        app.manage(crate::agent_history::AgentHistoryState::default());
        let executor = TauriBackendToolExecutor {
            app: app.clone(),
            snapshot: None,
            web_search_tracker: Mutex::new(Default::default()),
            control: RequestControl::new(None),
            journal: Arc::new(Default::default()),
            idempotency_key: "key-1".into(),
            events: None,
            live_status: false,
            request_images: Vec::new(),
            attachment_images: Vec::new(),
        };
        let context = BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
            channel: BackendChannel::Telegram,
            scope: BackendScope::Library,
            persistence_policy: PersistencePolicy::Persistent,
        };
        let call = |name: &str, arguments: Value| ToolCall { id: format!("call-{name}"), name: name.into(), arguments, round: 1 };

        // Telegram has no open view: the context says so, it does not fail.
        let workspace = executor.execute(&context, &call("get_workspace_context", json!({}))).expect("context");
        assert!(workspace.ok);
        assert_eq!(workspace.data.as_ref().expect("data")["available"], false);

        // An operation the backend does not know says why.
        let unknown = executor
            .execute(&context, &call("preview_markdown_edit", json!({ "path": "Meet.md", "operation": { "type": "replaceParagraph", "payload": {} } })))
            .expect_err("unknown operation");
        assert!(unknown.message.contains("replaceParagraph") && unknown.message.contains("replaceText"), "{}", unknown.message);

        let revision = crate::backend::compute_document_revision(source);
        let edit = json!({
            "path": "Meet.md",
            "expectedRevision": revision,
            "operation": { "type": "replaceText", "payload": {
                "find": "La extensión del horario de remesas de 0 a 3.",
                "replace": "Restringir el horario de operatoria de 0 a 1, en corresponsalías y remesas.",
            } },
        });
        let preview = executor
            .execute(&context, &call("preview_markdown_edit", through_float(&edit)))
            .expect("preview")
            .data
            .expect("preview data");
        // The model sees what changes, not the whole note twice.
        assert_eq!(preview["path"], "Meet.md");
        assert_eq!(preview["hunks"].as_array().map(Vec::len), Some(1));
        assert!(preview.get("newContent").is_none() && preview.get("oldContent").is_none());
        // The apply takes the same path and operation, not the preview.
        let applied = executor
            .execute(&context, &call("apply_markdown_edit", through_float(&edit)))
            .expect("applied");
        assert!(applied.ok, "{:?}", applied.error);
        assert_eq!(
            std::fs::read_to_string(root.join("Meet.md")).expect("note"),
            "# Meet\n\n- Restringir el horario de operatoria de 0 a 1, en corresponsalías y remesas.\n- Otro punto.\n"
        );
        // The same revision again is stale now.
        let stale = executor
            .execute(&context, &call("apply_markdown_edit", through_float(&edit)))
            .expect_err("stale revision");
        assert_eq!(stale.code, notia_backend_core::BackendErrorCode::Conflict);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// `improve.md`: the agent notes what Notia could improve, once each,
    /// only for the owner, and no agent file the prompt reads changes.
    #[test]
    fn the_agent_notes_improvements_for_notia_once() {
        use crate::host::{AppContext, AppPaths, HostPorts, Manager};
        use notia_backend_core::{
            BackendActor, BackendChannel, BackendErrorCode, BackendRequestContext, BackendScope, PersistencePolicy,
            RequestControl, ToolCall, ToolExecutor,
        };
        use serde_json::json;
        use std::sync::{Arc, Mutex};

        let root = std::env::temp_dir().join(format!("notia-agent-improve-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".agent/memory")).expect("root");
        let app = AppContext::new(AppPaths::default(), HostPorts::default());
        let registry = crate::library_registry::LibraryBindingRegistry::default();
        registry.register_desktop_root("library-1", &root).expect("binding");
        app.manage(registry);
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        app.manage(crate::agent_history::AgentHistoryState::default());
        app.manage(crate::agent_workspace::AgentWorkspaceState::prepared_for("library-1"));
        let executor = TauriBackendToolExecutor {
            app: app.clone(),
            snapshot: None,
            web_search_tracker: Mutex::new(Default::default()),
            control: RequestControl::new(None),
            journal: Arc::new(Default::default()),
            idempotency_key: "key-1".into(),
            events: None,
            live_status: false,
            request_images: Vec::new(),
            attachment_images: Vec::new(),
        };
        let mut context = BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
            channel: BackendChannel::App,
            scope: BackendScope::Library,
            persistence_policy: PersistencePolicy::Persistent,
        };
        let note = |kind: &str, text: &str| ToolCall {
            id: "call-improve".into(),
            name: "add_agent_improvement".into(),
            arguments: json!({ "kind": kind, "improvement": text }),
            round: 1,
        };
        let improve = || std::fs::read_to_string(root.join(".agent/memory/improve.md")).expect("improve.md");

        let added = executor
            .execute(&context, &note("error", "apply_multi_document_markdown_edit rechaza siempre el preview."))
            .expect("noted");
        assert_eq!(added.data.expect("data")["changed"], true);
        let repeated = executor
            .execute(&context, &note("error", "apply_multi_document_markdown_edit rechaza siempre el preview."))
            .expect("repeated");
        assert_eq!(repeated.data.expect("data")["changed"], false);
        executor.execute(&context, &note("herramienta", "Falta renombrar carpetas.")).expect("tool");

        let file = improve();
        assert!(file.contains("contexto: \"#System\""), "{file}");
        assert!(file.contains("# Mejoras para Notia"));
        assert_eq!(file.matches(" · error · apply_multi_document_markdown_edit rechaza siempre el preview.").count(), 1, "{file}");
        assert!(file.contains(" · herramienta · Falta renombrar carpetas."));
        // Nothing the prompt reads was written.
        let written = std::fs::read_dir(root.join(".agent/memory")).expect("memory").count();
        assert_eq!(written, 1);

        context.actor.library_user_id = "user-guest".into();
        let guest = executor.execute(&context, &note("mejora", "Otra.")).expect_err("guest");
        assert_eq!(guest.code, BackendErrorCode::Forbidden);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    /// The chat request of 2026-10-25: chain a course by setting `nextPage`
    /// and `previousPage` of many documents in one confirmed apply.
    #[test]
    fn the_agent_sets_several_properties_of_several_documents_in_one_apply() {
        use crate::host::{AppContext, AppPaths, HostPorts, Manager};
        use notia_backend_core::{
            BackendActor, BackendChannel, BackendErrorCode, BackendRequestContext, BackendScope, ConfirmationDecision,
            MutationPreview, PersistencePolicy, RequestControl, ToolCall, ToolExecutor,
        };
        use serde_json::json;
        use std::sync::{Arc, Mutex};

        let root = std::env::temp_dir().join(format!("notia-agent-chain-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Cursos")).expect("root");
        for name in ["Intro", "Algoritmos"] {
            std::fs::write(root.join(format!("Cursos/{name}.md")), format!("---\nnextPage: \"[[viejo.md]]\"\n---\n# {name}\n")).expect("note");
        }
        let app = AppContext::new(AppPaths::default(), HostPorts::default());
        let registry = crate::library_registry::LibraryBindingRegistry::default();
        registry.register_desktop_root("library-1", &root).expect("binding");
        app.manage(registry);
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        app.manage(crate::agent_history::AgentHistoryState::default());
        let executor = TauriBackendToolExecutor {
            app: app.clone(),
            snapshot: None,
            web_search_tracker: Mutex::new(Default::default()),
            control: RequestControl::new(None),
            journal: Arc::new(Default::default()),
            idempotency_key: "key-1".into(),
            events: None,
            live_status: false,
            request_images: Vec::new(),
            attachment_images: Vec::new(),
        };
        let context = BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
            channel: BackendChannel::App,
            scope: BackendScope::Library,
            persistence_policy: PersistencePolicy::Persistent,
        };
        let set = |path: &str, key: &str, value: &str| json!({
            "path": path,
            "operation": { "type": "setFrontmatter", "payload": { "key": key, "value": value } },
        });
        let call = ToolCall {
            id: "call-chain".into(),
            name: "apply_multi_document_markdown_edit".into(),
            arguments: json!({ "edits": [
                set("Cursos/Intro.md", "nextPage", "[[Cursos/Algoritmos.md]]"),
                set("Cursos/Intro.md", "previousPage", "N/A"),
                set("Cursos/Algoritmos.md", "nextPage", "N/A"),
                set("Cursos/Algoritmos.md", "previousPage", "[[Cursos/Intro.md]]"),
            ] }),
            round: 1,
        };
        let accept = |preview: &MutationPreview| ConfirmationDecision {
            operation_id: preview.operation_id.clone(),
            accepted: true,
            hunk_ids: Vec::new(),
            approve_all: false,
            suggestion: None,
        };

        // A document that changes while the person decides is not written.
        let preview = executor.preview(&context, &call).expect("preview").expect("a preview");
        assert_eq!(preview.documents.len(), 2);
        std::fs::write(root.join("Cursos/Algoritmos.md"), "---\nnextPage: N/A\n---\n# Algoritmos editado\n").expect("edit");
        let stale = executor.execute_confirmed(&context, &call, &accept(&preview), Some(&preview)).expect_err("stale");
        assert_eq!(stale.code, BackendErrorCode::Conflict);
        assert!(std::fs::read_to_string(root.join("Cursos/Intro.md")).expect("intro").contains("viejo.md"));

        let preview = executor.preview(&context, &call).expect("preview").expect("a preview");
        let result = executor.execute_confirmed(&context, &call, &accept(&preview), Some(&preview)).expect("applied");
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(
            std::fs::read_to_string(root.join("Cursos/Intro.md")).expect("intro"),
            "---\nnextPage: \"[[Cursos/Algoritmos.md]]\"\npreviousPage: N/A\n---\n# Intro\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("Cursos/Algoritmos.md")).expect("algoritmos"),
            "---\nnextPage: N/A\npreviousPage: \"[[Cursos/Intro.md]]\"\n---\n# Algoritmos editado\n"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn task_group_confirmations_name_the_groups() {
        use notia_backend_core::{TaskGroupDto, TaskMutationDto};
        let group = |id: &str, name: &str| TaskGroupDto {
            library_id: "lib".into(),
            group_id: id.into(),
            board_id: "default".into(),
            name: name.into(),
            color: "#10b981".into(),
            revision: 1,
            order: 0,
            kind: None,
        };
        let groups = [group("legacy-group-1", "Backlog Q"), group("legacy-group-2", "Sprint Actual")];
        let summary = |mutation: TaskMutationDto| super::task_mutation_summary(&mutation, &groups, &[]);
        assert_eq!(
            summary(TaskMutationDto::ReorderGroups {
                board_id: "default".into(),
                group_ids: vec!["legacy-group-2".into(), "legacy-group-1".into()],
            }),
            "Ordenar los grupos: «Sprint Actual» → «Backlog Q»."
        );
        assert_eq!(
            summary(TaskMutationDto::UpdateGroup {
                board_id: "default".into(),
                group_id: "legacy-group-1".into(),
                name: "Backlog Q actual".into(),
                color: "#10b981".into(),
                column_kind: None,
            }),
            "Cambiar el grupo «Backlog Q»: nombre «Backlog Q actual»."
        );
        assert_eq!(
            summary(TaskMutationDto::MoveTicket { ticket_id: "t-1".into(), group_id: Some("legacy-group-2".into()) }),
            "Mover el ticket t-1 al grupo «Sprint Actual»."
        );
    }

    #[test]
    fn comment_confirmations_quote_the_comment_whole() {
        use notia_backend_core::{TaskCommentDto, TaskMutationDto};
        let comments = [TaskCommentDto {
            library_id: "lib".into(),
            comment_id: "c-1".into(),
            ticket_id: "t-1".into(),
            author_user_id: "user-owner".into(),
            body: "Doris finalizó los 3, listos para prueba\n".into(),
            created_at_unix_ms: 1,
            revision: 1,
            logical_path: String::new(),
        }];
        let summary = |mutation: TaskMutationDto| super::task_mutation_summary(&mutation, &[], &comments);
        assert_eq!(
            summary(TaskMutationDto::UpdateComment {
                ticket_id: "t-1".into(),
                comment_id: "c-1".into(),
                body: "Doris finalizó 2 de los 3".into(),
            }),
            "Cambiar el comentario «Doris finalizó los 3, listos para prueba» del ticket t-1 por «Doris finalizó 2 de los 3»."
        );
        assert_eq!(
            summary(TaskMutationDto::DeleteComment { ticket_id: "t-1".into(), comment_id: "c-1".into() }),
            "Eliminar el comentario «Doris finalizó los 3, listos para prueba» del ticket t-1."
        );
    }

    /// Board indexes are rebuilt by the store: an agent document tool must
    /// never write them, while tickets and other notes stay writable.
    #[test]
    fn agent_document_writes_skip_task_manager_indexes() {
        use notia_backend_core::{BackendErrorCode, DocumentLocatorDto};
        let locator = |path: &str| DocumentLocatorDto::new("lib", path, None, None).expect("locator");
        let refused = TauriBackendToolExecutor::ensure_agent_writable(&locator("task-mannager/defaultTaskIndex.md"))
            .expect_err("index");
        assert_eq!(refused.code, BackendErrorCode::Forbidden);
        assert!(TauriBackendToolExecutor::ensure_agent_writable(&locator("task-mannager/default/Cobranzas integradas.md")).is_ok());
        assert!(TauriBackendToolExecutor::ensure_agent_writable(&locator("Personal/notas.md")).is_ok());
        // Recipes change only through the recipe tools, which review them.
        let recipe = TauriBackendToolExecutor::ensure_agent_writable(&locator("recipes/Guiso de lentejas.md")).expect_err("recipe");
        assert_eq!(recipe.code, BackendErrorCode::Forbidden);
    }

    #[test]
    fn a_failed_request_logs_its_tools_by_name_without_their_data() {
        use notia_backend_core::BackendEvent;
        let completed = |tool_name: &str, ok: bool| BackendEvent::ToolCompleted {
            request_id: "r1".into(),
            tool_name: tool_name.into(),
            round: 1,
            ok,
            changed: Some(ok),
            operation_id: None,
        };
        let events = [
            completed("search_gmail_messages", true),
            completed("trash_gmail_messages", true),
            completed("search_gmail_messages", true),
            completed("trash_gmail_messages", false),
            BackendEvent::AssistantNote { request_id: "r1".into(), text: "Borro los correos de Jumbo.".into() },
        ];
        assert_eq!(super::tool_usage(events.iter()), "search_gmail_messages 2, trash_gmail_messages 2 (1 falló)");
        assert_eq!(super::tool_usage(std::iter::empty()), "ninguna");
    }

    /// Images stay in memory for the run; the saved journal keeps the text.
    #[test]
    fn the_saved_journal_record_leaves_the_images_out() {
        use notia_backend_core::chat_attachments::{MessageAttachment, MessageAttachmentKind};
        use notia_backend_core::{BackendMessage, MessageRole, ProviderMessage, ProviderMessageRole};
        let image = MessageAttachment {
            name: "foto-1.jpg".into(),
            media_type: "image/jpeg".into(),
            kind: MessageAttachmentKind::Image,
            pages: vec!["QUJD".repeat(1_000)],
            text_content: None,
            extracted_text: None,
            page_count: None,
        };
        let request: notia_backend_core::AgentRequest = serde_json::from_value(serde_json::json!({
            "context": {
                "requestId": "r", "libraryId": "library-1",
                "actor": { "libraryUserId": "user-owner" },
                "channel": "telegram", "scope": "library", "persistencePolicy": "persistent"
            },
            "messages": [], "tools": [], "idempotencyKey": "k"
        }))
        .expect("request");
        let mut request = request;
        request.messages.push(BackendMessage {
            role: MessageRole::User,
            content: "cargá estos tickets".into(),
            images: vec!["QUJD".into()],
            attachments: vec![image],
        });
        let record = super::JournalRecord {
            request: Some(request),
            continuation: Some(notia_backend_core::AgentContinuation {
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: "cargá estos tickets".into(),
                    images: vec!["QUJD".into()],
                    tool_calls: Vec::new(),
                    tool_name: None,
                }],
                rounds: 1,
                pending_call: serde_json::from_value(serde_json::json!({"id": "c", "name": "create_finance_purchase", "arguments": {}, "round": 1})).expect("call"),
                tool_results: Vec::new(),
                preview: None,
                approve_all: false,
                tool_areas: None,
            }),
            ..Default::default()
        };
        let saved = record.without_binary_content();
        let message = &saved.request.as_ref().expect("request").messages[0];
        assert_eq!(message.content, "cargá estos tickets");
        assert!(message.images.is_empty() && message.attachments.is_empty());
        assert!(saved.continuation.as_ref().expect("continuation").messages[0].images.is_empty());
        assert!(!serde_json::to_string(&saved).expect("json").contains("QUJD"));
    }

    /// Every turn sends the supported tools the scope allows (the library
    /// chat, routed by area); a scope that outgrows the limit fails every run
    /// with «El request supera el límite de tools» (Finanzas did, with 77,
    /// while the limit was 64).
    #[test]
    fn every_scope_fits_the_tool_limit_for_the_owner() {
        use notia_backend_core::{
            AuthorizationPrincipal, BackendActor, BackendChannel, BackendLimits, BackendRequestContext, BackendScope,
            PersistencePolicy, ToolAccess, ToolCatalogProjection,
        };
        let names = TauriBackendToolExecutor::supported_tool_names().iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let principal = AuthorizationPrincipal {
            library_id: "library-1".into(),
            library_user_id: "user-owner".into(),
            allowed_contexts: Vec::new(),
            all_contexts: true,
        };
        for scope in [BackendScope::Library, BackendScope::Finance, BackendScope::Document, BackendScope::TaskManager, BackendScope::Graph] {
            let context = BackendRequestContext {
                request_id: "request-1".into(),
                library_id: "library-1".into(),
                actor: BackendActor { library_user_id: "user-owner".into(), external_identity: None },
                channel: BackendChannel::App,
                scope: scope.clone(),
                persistence_policy: PersistencePolicy::Persistent,
            };
            let tools = notia_backend_core::restrict_tool_access(
                notia_backend_core::project_canonical_tool_catalog(&context, &principal, &names, ToolCatalogProjection::Full).expect("catalog"),
                ToolAccess::All,
                true,
            );
            let max = BackendLimits::default().max_tools;
            // Every chat is routed: each area it offers fits in a round with
            // the tools every round keeps and `change_tool_areas`.
            use notia_backend_core::tool_routing::{offered_areas, tool_area, turn_tools};
            assert!(tools.len() <= notia_backend_core::MAX_TOOL_POOL, "{scope:?}");
            for area in offered_areas(&tools) {
                let round = turn_tools(&tools, &[area], max);
                let all = tools.iter().filter(|tool| tool_area(&tool.name) == Some(area)).count();
                let kept = round.iter().filter(|tool| tool_area(&tool.name) == Some(area)).count();
                assert!(round.len() <= max && kept == all, "{scope:?} {area:?}: {} tools", round.len());
            }
        }
    }

    /// A mail tool in the catalog without an adapter would reach the model
    /// and fail on every call.
    #[test]
    fn every_mail_tool_of_the_catalog_has_an_adapter() {
        for tool in notia_backend_core::mail_tools::mail_tool_contracts() {
            assert!(TauriBackendToolExecutor::supported_tool_names().contains(&tool.name.as_str()), "{}", tool.name);
        }
    }

    /// A supported tool missing from the canonical catalog makes every run
    /// fail during projection; one without an argument schema is unusable
    /// by the model.
    #[test]
    fn every_supported_tool_is_in_the_catalog_with_an_argument_schema() {
        let catalog = notia_backend_core::canonical_tool_catalog();
        for name in TauriBackendToolExecutor::supported_tool_names() {
            let tool = catalog
                .iter()
                .find(|tool| tool.name == *name)
                .unwrap_or_else(|| panic!("tool missing from catalog: {name}"));
            assert!(
                tool.input_schema["properties"].is_object(),
                "tool without argument schema: {name}"
            );
        }
    }
}
