//! Tauri-independent coordination for operations that need a user decision.
//!
//! This layer owns protocol state transitions only. Domain adapters provide
//! previews, revisions and undo through the ports; accepting a decision never
//! turns a missing preview into permission to mutate.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use super::agent::{
    run_agent_with_interaction, AgentProvider, AgentRunResult, AgentRuntimeOptions,
    AgentStateHooks, ToolExecutor,
};
use super::control::{RequestControl, RequestControlRegistry};
use super::error::{BackendError, BackendErrorCode};
use super::interaction::{
    validate_clarification_answer, validate_confirmation, OperationGenerationCache,
    OperationReview, OperationReviewPort, OperationStatePort, OperationToken, PlanStatus,
};
use super::protocol::{
    BackendLimits, BackendRequest, BackendRequestEnvelope, BackendResponse,
    BackendResponse as Response, BackendResponseEnvelope, CancelRequest, GetOperationRequest,
    OperationState, OperationStatus, PendingInteraction, PlanDecision, ProtocolVersion,
    ResumeDecision, ResumeRequest, ReviewRequest, UndoOperationRequest,
};
use super::{BackendRequestContext, RevisionPort};

/// Coordinates persisted checkpoints and cancellation without owning a
/// provider, executor or platform handle.
pub struct InteractionRuntime<'a> {
    state: &'a dyn OperationStatePort,
    review: Option<&'a dyn OperationReviewPort>,
    revisions: Option<&'a dyn RevisionPort>,
    generations: &'a OperationGenerationCache,
    controls: Arc<RequestControlRegistry>,
}

impl<'a> InteractionRuntime<'a> {
    pub fn new(
        state: &'a dyn OperationStatePort,
        review: Option<&'a dyn OperationReviewPort>,
        revisions: Option<&'a dyn RevisionPort>,
        generations: &'a OperationGenerationCache,
    ) -> Self {
        Self {
            state,
            review,
            revisions,
            generations,
            controls: Arc::new(RequestControlRegistry::default()),
        }
    }

    /// Shares the host-owned registry so a cancel received by another
    /// transport call reaches the request that is executing.
    pub fn with_control_registry(mut self, controls: Arc<RequestControlRegistry>) -> Self {
        self.controls = controls;
        self
    }

    pub fn register_control(
        &self,
        context: &BackendRequestContext,
        control: RequestControl,
    ) -> Result<(), BackendError> {
        self.controls.register(context, control)
    }

    /// Releases the request-owned cancellation handle after a terminal run.
    /// Keeping this explicit prevents a long-lived backend from retaining
    /// controls for every request ever seen.
    pub fn unregister_control(
        &self,
        context: &BackendRequestContext,
    ) -> Result<(), BackendError> {
        self.controls.unregister(context)
    }

    pub fn run_agent_request(
        &self,
        provider: &dyn AgentProvider,
        executor: &dyn ToolExecutor,
        agent_state: &dyn AgentStateHooks,
        events: &dyn super::BackendEventSink,
        request: &super::AgentRequest,
        principal: &super::AuthorizationPrincipal,
        control: &RequestControl,
        options: &AgentRuntimeOptions,
    ) -> Result<BackendResponse, BackendError> {
        self.register_control(&request.context, control.clone())?;
        let result = match run_agent_with_interaction(
            provider,
            executor,
            agent_state,
            events,
            request,
            principal,
            control,
            options,
            self,
        ) {
            Ok(AgentRunResult::Completed(response)) => Ok(BackendResponse::Result { response }),
            Ok(AgentRunResult::Waiting(status)) => Ok(BackendResponse::Operation { status }),
            Err(error) => Err(error),
        };
        self.finish_operation(&request.context, &request.idempotency_key, &result);
        if !matches!(&result, Ok(BackendResponse::Operation { .. })) {
            self.unregister_control(&request.context)?;
        }
        result
    }

    pub fn run_agent_with_resume(
        &self,
        provider: &dyn AgentProvider,
        executor: &dyn ToolExecutor,
        agent_state: &dyn AgentStateHooks,
        events: &dyn super::BackendEventSink,
        request: &super::AgentRequest,
        principal: &super::AuthorizationPrincipal,
        control: &RequestControl,
        options: &AgentRuntimeOptions,
        operation_id: &str,
        decision: super::ResumeDecision,
    ) -> Result<super::AgentRunResult, BackendError> {
        let result = super::agent::run_agent_with_resume(
            provider,
            executor,
            agent_state,
            events,
            request,
            principal,
            control,
            options,
            self,
            operation_id,
            decision,
        );
        self.finish_agent_result(&request.context, &request.idempotency_key, &result);
        result
    }

    fn finish_agent_result(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        result: &Result<super::AgentRunResult, BackendError>,
    ) {
        let state = match result {
            Ok(super::AgentRunResult::Completed(response)) => {
                Some((OperationState::Completed, response.changed))
            }
            Ok(super::AgentRunResult::Waiting(_)) => None,
            Err(error) if error.code == BackendErrorCode::Conflict => None,
            Err(error) if error.code == BackendErrorCode::Cancelled => {
                Some((OperationState::Cancelled, false))
            }
            Err(_) => Some((OperationState::Failed, false)),
        };
        if let Some((state, changed)) = state {
            self.finish_operation_state(context, idempotency_key, state, changed);
        }
    }

    fn finish_operation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        result: &Result<BackendResponse, BackendError>,
    ) {
        let state = match result {
            Ok(BackendResponse::Result { response }) => {
                Some((OperationState::Completed, response.changed))
            }
            Ok(BackendResponse::Operation { .. }) => None,
            Err(error) if error.code == BackendErrorCode::Conflict => None,
            Err(error) if error.code == BackendErrorCode::Cancelled => {
                Some((OperationState::Cancelled, false))
            }
            Err(_) => Some((OperationState::Failed, false)),
            _ => None,
        };
        if let Some((state, changed)) = state {
            self.finish_operation_state(context, idempotency_key, state, changed);
        }
    }

    fn finish_operation_state(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        state: OperationState,
        changed: bool,
    ) {
        let Ok(Some(mut review)) = self
            .state
            .load_operation_for_request(context, idempotency_key)
        else {
            return;
        };
        review.state = state;
        review.changed = changed;
        let _ = self.state.store_operation(context, idempotency_key, &review);
    }

    pub fn begin_waiting(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        review: OperationReview,
        last_event_sequence: u64,
    ) -> Result<BackendResponse, BackendError> {
        context.validate()?;
        review.operation.validate()?;
        if let Some(preview) = &review.preview {
            if preview.operation_id != review.operation.operation_id {
                return Err(stale_error("El preview no corresponde a la operación."));
            }
        }
        if let Some(clarification) = &review.clarification {
            clarification.validate()?;
        }
        if let Some(plan) = &review.plan {
            plan.validate()?;
        }
        if let Some(existing) = self
            .state
            .load_operation_for_request(context, idempotency_key)?
            .filter(|existing| is_waiting(existing.state))
        {
            // Re-entering the same pending interaction is idempotent. A
            // different pending interaction must be resolved first; finished
            // or resumed interactions are replaced by the next checkpoint.
            if existing.operation.operation_id != review.operation.operation_id {
                return Err(stale_error(
                    "La solicitud ya tiene otra interacción pendiente.",
                ));
            }
            return Ok(BackendResponse::Operation {
                status: status_from_review(
                    context.request_id.clone(),
                    last_event_sequence,
                    &existing,
                ),
            });
        }
        self.generations.observe(
            &scoped_operation_key(context, &review.operation.operation_id),
            review.operation.generation,
        )?;
        self.state
            .store_operation(context, idempotency_key, &review)?;
        Ok(BackendResponse::Operation {
            status: status_from_review(context.request_id.clone(), last_event_sequence, &review),
        })
    }

    pub fn next_operation_token(
        &self,
        context: &BackendRequestContext,
        operation_id: impl Into<String>,
    ) -> OperationToken {
        let operation_id = operation_id.into();
        OperationToken {
            generation: self
                .generations
                .next_generation(&scoped_operation_key(context, &operation_id)),
            operation_id,
        }
    }

    pub fn status_for_operation(
        &self,
        context: &BackendRequestContext,
        operation: &OperationToken,
        idempotency_key: &str,
        last_event_sequence: u64,
    ) -> Result<OperationStatus, BackendError> {
        let review = self.load_matching(context, operation, idempotency_key)?;
        Ok(status_from_review(
            context.request_id.clone(),
            last_event_sequence,
            &review,
        ))
    }

    pub fn existing_status_for_request(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        last_event_sequence: u64,
    ) -> Result<Option<OperationStatus>, BackendError> {
        self.state
            .load_operation_for_request(context, idempotency_key)
            .map(|review| {
                review.map(|review| {
                    status_from_review(context.request_id.clone(), last_event_sequence, &review)
                })
            })
    }

    pub fn begin_confirmation(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        tool: &super::ToolDefinition,
        call: &super::ToolCall,
        last_event_sequence: u64,
    ) -> Result<BackendResponse, BackendError> {
        self.begin_confirmation_with_preview(
            context,
            idempotency_key,
            tool,
            call,
            None,
            last_event_sequence,
        )
    }

    pub fn begin_confirmation_with_preview(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        tool: &super::ToolDefinition,
        call: &super::ToolCall,
        supplied_preview: Option<super::protocol::MutationPreview>,
        last_event_sequence: u64,
    ) -> Result<BackendResponse, BackendError> {
        let operation = self.next_operation_token(context, &call.id);
        let preview = match supplied_preview {
            Some(mut preview) => {
                preview.operation_id = operation.operation_id.clone();
                Some(preview)
            }
            None => match self.review {
                Some(review) => review.prepare_operation(context, &operation, tool, call)?,
                None => None,
            },
        };
        // A confirmation always shows something: without a preview the
        // clients got a waiting operation with nothing to ask and ended the
        // turn with «no devolvió una respuesta final».
        let preview = preview.or_else(|| Some(call_preview(&operation, call)));
        self.begin_waiting(
            context,
            idempotency_key,
            OperationReview {
                operation,
                state: OperationState::WaitingConfirmation,
                changed: false,
                can_undo: false,
                preview,
                clarification: None,
                plan: None,
            },
            last_event_sequence,
        )
    }

    pub fn begin_clarification(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        clarification: super::interaction::ClarificationRequest,
        last_event_sequence: u64,
    ) -> Result<BackendResponse, BackendError> {
        clarification.validate()?;
        let review = OperationReview {
            operation: clarification.operation.clone(),
            state: OperationState::WaitingClarification,
            changed: false,
            can_undo: false,
            preview: None,
            clarification: Some(clarification),
            plan: None,
        };
        self.begin_waiting(context, idempotency_key, review, last_event_sequence)
    }

    pub fn begin_plan(
        &self,
        context: &BackendRequestContext,
        idempotency_key: &str,
        operation: OperationToken,
        plan: super::ExecutionPlan,
        last_event_sequence: u64,
    ) -> Result<BackendResponse, BackendError> {
        plan.validate()?;
        let review = OperationReview {
            operation,
            state: OperationState::WaitingPlan,
            changed: false,
            can_undo: false,
            preview: None,
            clarification: None,
            plan: Some(plan),
        };
        self.begin_waiting(context, idempotency_key, review, last_event_sequence)
    }

    pub fn handle_resume(&self, request: &ResumeRequest) -> Result<BackendResponse, BackendError> {
        request.context.validate()?;
        request.operation.validate()?;
        let stored = self.load_matching(
            &request.context,
            &request.operation,
            &request.idempotency_key,
        )?;
        let next_state = self.validate_decision(request, &stored)?;
        let mut resumed = stored;
        resumed.state = next_state;
        self.state
            .store_operation(&request.context, &request.idempotency_key, &resumed)?;
        Ok(Response::Resumed {
            status: status_from_review(
                request.request_id.clone(),
                request.last_event_sequence,
                &resumed,
            ),
            decision: request.decision.clone(),
        })
    }

    /// Cancels a waiting operation, a running request, or both. Without an
    /// operation token the cancel targets the active run of the scoped
    /// request, which lets a client stop the provider/tool loop before any
    /// interaction exists.
    pub fn handle_cancel(&self, request: &CancelRequest) -> Result<BackendResponse, BackendError> {
        let stored = match &request.operation {
            Some(operation) => Some(self.load_matching(
                &request.context,
                operation,
                &request.idempotency_key,
            )?),
            None => self
                .state
                .load_operation_for_request(&request.context, &request.idempotency_key)?,
        };
        let pending = stored.filter(|review| !is_terminal(review.state));
        if request.operation.is_some() && pending.is_none() {
            return Err(BackendError::new(
                BackendErrorCode::Conflict,
                "La operación ya terminó y no puede cancelarse.",
                false,
            ));
        }
        let cancelled_run = self.controls.cancel(&request.context)?;
        if pending.is_none() && !cancelled_run {
            return Err(BackendError::new(
                BackendErrorCode::NotFound,
                "No hay una solicitud activa para cancelar.",
                false,
            ));
        }
        if let Some(mut cancelled) = pending {
            cancelled.state = OperationState::Cancelled;
            self.state
                .store_operation(&request.context, &request.idempotency_key, &cancelled)?;
        }
        Ok(BackendResponse::Cancelled {
            request_id: request.request_id.clone(),
        })
    }

    /// Returns the state of an operation. Without an operation token it
    /// reports the latest interaction of the request, or `running` while the
    /// scoped run is active, so a reconnecting client can re-attach by
    /// request identity and replay events after its last sequence.
    pub fn handle_get_operation(
        &self,
        request: &GetOperationRequest,
    ) -> Result<BackendResponse, BackendError> {
        let stored = match &request.operation {
            Some(operation) => Some(self.load_matching(
                &request.context,
                operation,
                &request.idempotency_key,
            )?),
            None => self
                .state
                .load_operation_for_request(&request.context, &request.idempotency_key)?,
        };
        let running = self.controls.is_running(&request.context)?;
        let status = match stored {
            Some(review) if !running || !is_terminal(review.state) => status_from_review(
                request.request_id.clone(),
                request.last_event_sequence,
                &review,
            ),
            _ if running => OperationStatus {
                request_id: request.request_id.clone(),
                operation: None,
                state: OperationState::Running,
                last_event_sequence: request.last_event_sequence,
                response: None,
                interaction: None,
                review: None,
            },
            _ => {
                return Err(BackendError::new(
                    BackendErrorCode::NotFound,
                    "La operación no existe.",
                    false,
                ))
            }
        };
        Ok(BackendResponse::Operation { status })
    }

    pub fn handle_review(&self, request: &ReviewRequest) -> Result<BackendResponse, BackendError> {
        let stored = self.load_matching(
            &request.context,
            &request.operation,
            &request.idempotency_key,
        )?;
        let review = self
            .review
            .ok_or_else(|| unsupported("La revisión de operaciones no está configurada."))?
            .review_operation(&request.context, &stored.operation)?;
        Ok(BackendResponse::Review { review })
    }

    pub fn handle_undo(
        &self,
        request: &UndoOperationRequest,
    ) -> Result<BackendResponse, BackendError> {
        let stored = self.load_matching(
            &request.context,
            &request.operation,
            &request.idempotency_key,
        )?;
        if !stored.can_undo || stored.state != OperationState::Completed {
            return Err(BackendError::new(
                BackendErrorCode::Conflict,
                "La operación no puede deshacerse en su estado actual.",
                false,
            ));
        }
        let result = self
            .review
            .ok_or_else(|| unsupported("El undo de operaciones no está configurado."))?
            .undo_operation(
                &request.context,
                &super::interaction::UndoRequest {
                    operation: request.operation.clone(),
                },
            )?;
        Ok(BackendResponse::Undo { result })
    }

    pub fn handle_request(
        &self,
        request: &BackendRequest,
        limits: &BackendLimits,
    ) -> Result<Option<BackendResponse>, BackendError> {
        request.validate(limits)?;
        match request {
            BackendRequest::Resume(request) => self.handle_resume(request).map(Some),
            BackendRequest::Cancel(request) => self.handle_cancel(request).map(Some),
            BackendRequest::GetOperation(request) => self.handle_get_operation(request).map(Some),
            BackendRequest::Review(request) => self.handle_review(request).map(Some),
            BackendRequest::Undo(request) => self.handle_undo(request).map(Some),
            BackendRequest::Run(_) => Ok(None),
        }
    }

    pub fn handle_envelope(
        &self,
        envelope: &BackendRequestEnvelope,
        limits: &BackendLimits,
    ) -> Result<Option<BackendResponseEnvelope>, BackendError> {
        envelope.protocol_version.validate()?;
        self.handle_request(&envelope.request, limits)
            .map(|response| {
                response.map(|response| BackendResponseEnvelope {
                    protocol_version: ProtocolVersion::default(),
                    response,
                })
            })
    }

    fn load_matching(
        &self,
        context: &BackendRequestContext,
        operation: &OperationToken,
        idempotency_key: &str,
    ) -> Result<OperationReview, BackendError> {
        operation.validate()?;
        let review = self
            .state
            .load_operation(context, &operation.operation_id, idempotency_key)?
            .or(self
                .state
                .load_operation_for_request(context, idempotency_key)?)
            .ok_or_else(|| {
                BackendError::new(BackendErrorCode::NotFound, "La operación no existe.", false)
            })?;
        if review.operation != *operation {
            return Err(stale_error(
                "La continuación pertenece a una generación obsoleta.",
            ));
        }
        self.generations.observe(
            &scoped_operation_key(context, &operation.operation_id),
            operation.generation,
        )?;
        Ok(review)
    }

    fn validate_decision(
        &self,
        request: &ResumeRequest,
        stored: &OperationReview,
    ) -> Result<OperationState, BackendError> {
        match (&request.decision, stored.state) {
            (ResumeDecision::Clarification(answer), OperationState::WaitingClarification) => {
                let clarification = stored.clarification.as_ref().ok_or_else(|| {
                    stale_error("La aclaración almacenada ya no está disponible.")
                })?;
                validate_clarification_answer(clarification, answer)?;
                Ok(OperationState::Running)
            }
            (ResumeDecision::Confirmation(decision), OperationState::WaitingConfirmation) => {
                let preview = stored.preview.as_ref().ok_or_else(|| {
                    stale_error("La confirmación no tiene un preview almacenado.")
                })?;
                let confirmation = super::interaction::ConfirmationRequest {
                    operation: stored.operation.clone(),
                    preview: preview.clone(),
                };
                let revisions = current_revisions(self.revisions, &request.context, preview)?;
                validate_confirmation(decision, &confirmation, &revisions)?;
                // A proposal instead of the change («Proponer otra cosa»)
                // goes back to the agent, which continues with it.
                if decision.accepted || has_text(decision.suggestion.as_deref()) {
                    Ok(OperationState::Running)
                } else {
                    Ok(OperationState::Cancelled)
                }
            }
            (ResumeDecision::Plan(decision), OperationState::WaitingPlan) => {
                validate_plan_decision(stored, decision)?;
                if decision.accepted || has_text(decision.suggestion.as_deref()) {
                    Ok(OperationState::Running)
                } else {
                    Ok(OperationState::Cancelled)
                }
            }
            _ => Err(stale_error(
                "La decisión no corresponde al estado almacenado.",
            )),
        }
    }
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

/// The tool and its arguments, for a confirmation whose tool has no preview
/// of its own.
fn call_preview(operation: &OperationToken, call: &super::ToolCall) -> super::protocol::MutationPreview {
    use super::protocol::{MutationPreview, MutationPreviewAction, PreviewHunk};
    MutationPreview {
        operation_id: operation.operation_id.clone(),
        summary: format!("Ejecutar {}", call.name),
        documents: Vec::new(),
        hunks: vec![PreviewHunk {
            id: operation.operation_id.clone(),
            document_path: call.name.clone(),
            start_line: 1,
            end_line: 1,
            old_text: String::new(),
            new_text: serde_json::to_string_pretty(&call.arguments).unwrap_or_default(),
        }],
        allowed_actions: vec![MutationPreviewAction::ApplyAll, MutationPreviewAction::Reject, MutationPreviewAction::Cancel],
    }
}

fn status_from_review(
    request_id: String,
    last_event_sequence: u64,
    review: &OperationReview,
) -> OperationStatus {
    let interaction = if let Some(value) = review.clarification.clone() {
        Some(PendingInteraction::Clarification(value))
    } else if let Some(preview) = review.preview.clone() {
        Some(PendingInteraction::Confirmation(
            super::interaction::ConfirmationRequest {
                operation: review.operation.clone(),
                preview,
            },
        ))
    } else if let Some(plan) = review.plan.clone() {
        Some(PendingInteraction::Plan(plan))
    } else {
        None
    };
    OperationStatus {
        request_id,
        operation: Some(review.operation.clone()),
        state: review.state,
        last_event_sequence,
        response: None,
        interaction,
        review: Some(review.clone()),
    }
}

fn current_revisions(
    revisions: Option<&dyn RevisionPort>,
    context: &BackendRequestContext,
    preview: &super::protocol::MutationPreview,
) -> Result<BTreeMap<String, u64>, BackendError> {
    let Some(revisions) = revisions else {
        if preview.documents.is_empty() {
            return Ok(BTreeMap::new());
        }
        return Err(stale_error("No se pudo verificar el preview almacenado."));
    };
    preview
        .documents
        .iter()
        .map(|document| {
            revisions
                .current_revision(context, &document.path)?
                .map(|revision| (document.path.clone(), revision))
                .ok_or_else(|| stale_error("La revisión del preview ya no existe."))
        })
        .collect()
}

fn validate_plan_decision(
    stored: &OperationReview,
    decision: &PlanDecision,
) -> Result<(), BackendError> {
    let plan = stored
        .plan
        .as_ref()
        .ok_or_else(|| stale_error("El plan almacenado ya no está disponible."))?;
    if plan.plan_id != decision.plan_id || plan.generation != decision.generation {
        return Err(stale_error("La decisión pertenece a un plan obsoleto."));
    }
    let known = plan
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<HashSet<_>>();
    if decision
        .step_ids
        .iter()
        .any(|id| !known.contains(id.as_str()))
        || has_duplicates(&decision.step_ids)
    {
        return Err(BackendError::invalid_input(
            "La decisión del plan contiene pasos desconocidos o repetidos.",
        ));
    }
    if plan.status != PlanStatus::AwaitingApproval {
        return Err(stale_error("El plan no está esperando aprobación."));
    }
    if let Some(suggestion) = &decision.suggestion {
        let suggestion = suggestion.trim();
        if decision.accepted
            || suggestion.is_empty()
            || suggestion.chars().count() > MAX_PLAN_SUGGESTION_CHARS
            || suggestion.chars().any(|character| character.is_control() && character != '\n' && character != '\t')
        {
            return Err(BackendError::invalid_input("La sugerencia del plan no es válida."));
        }
    }
    Ok(())
}

/// Longest suggestion of changes to a plan.
const MAX_PLAN_SUGGESTION_CHARS: usize = 4_000;

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn stale_error(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Conflict, message, true)
}

fn unsupported(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, message, false)
}

/// Operation ids come from provider tool calls and are not globally unique;
/// generations are therefore tracked per library and user.
fn scoped_operation_key(context: &BackendRequestContext, operation_id: &str) -> String {
    format!(
        "{}\0{}\0{}",
        context.library_id, context.actor.library_user_id, operation_id
    )
}

fn is_terminal(state: OperationState) -> bool {
    matches!(
        state,
        OperationState::Completed | OperationState::Cancelled | OperationState::Failed
    )
}

fn is_waiting(state: OperationState) -> bool {
    matches!(
        state,
        OperationState::WaitingClarification
            | OperationState::WaitingConfirmation
            | OperationState::WaitingPlan
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{
        ClarificationAnswer, ClarificationOption, ClarificationRequest, ExecutionPlan,
        OperationReview, PlanStep, UndoRequest, UndoResult,
    };
    use crate::protocol::{
        ConfirmationDecision, MutationPreview, MutationPreviewAction, PreviewDocument, PreviewHunk,
    };
    use crate::{
        BackendActor, BackendChannel, BackendScope, PersistencePolicy, ToolCall, ToolDefinition,
        ToolResult,
    };
    use std::sync::Mutex;

    fn context() -> BackendRequestContext {
        BackendRequestContext {
            request_id: "request-1".into(),
            library_id: "library-1".into(),
            actor: BackendActor {
                library_user_id: "user-owner".into(),
                external_identity: None,
            },
            channel: BackendChannel::App,
            scope: BackendScope::Library,
            persistence_policy: PersistencePolicy::Persistent,
        }
    }

    fn preview(operation_id: &str) -> MutationPreview {
        MutationPreview {
            operation_id: operation_id.into(),
            summary: "editar nota".into(),
            documents: vec![PreviewDocument {
                path: "note.md".into(),
                expected_revision: 4,
                current_revision: 4,
            }],
            hunks: vec![PreviewHunk {
                id: "hunk-1".into(),
                document_path: "note.md".into(),
                start_line: 1,
                end_line: 1,
                old_text: "old".into(),
                new_text: "new".into(),
            }],
            allowed_actions: vec![
                MutationPreviewAction::ApplyAll,
                MutationPreviewAction::ApplySelected,
                MutationPreviewAction::Reject,
            ],
        }
    }

    #[derive(Default)]
    struct State {
        review: Mutex<Option<OperationReview>>,
    }

    impl OperationStatePort for State {
        fn load_operation(
            &self,
            _: &BackendRequestContext,
            operation_id: &str,
            _: &str,
        ) -> Result<Option<OperationReview>, BackendError> {
            Ok(self
                .review
                .lock()
                .expect("review lock")
                .clone()
                .filter(|review| review.operation.operation_id == operation_id))
        }

        fn store_operation(
            &self,
            _: &BackendRequestContext,
            _: &str,
            review: &OperationReview,
        ) -> Result<(), BackendError> {
            *self.review.lock().expect("review lock") = Some(review.clone());
            Ok(())
        }

        fn load_operation_for_request(
            &self,
            _: &BackendRequestContext,
            _: &str,
        ) -> Result<Option<OperationReview>, BackendError> {
            Ok(self.review.lock().expect("review lock").clone())
        }
    }

    struct Ports;

    impl OperationReviewPort for Ports {
        fn prepare_operation(
            &self,
            _: &BackendRequestContext,
            operation: &OperationToken,
            _: &ToolDefinition,
            _: &ToolCall,
        ) -> Result<Option<MutationPreview>, BackendError> {
            Ok(Some(preview(&operation.operation_id)))
        }

        fn review_operation(
            &self,
            _: &BackendRequestContext,
            operation: &OperationToken,
        ) -> Result<OperationReview, BackendError> {
            Ok(OperationReview {
                operation: operation.clone(),
                state: OperationState::Completed,
                changed: true,
                can_undo: true,
                preview: Some(preview(&operation.operation_id)),
                clarification: None,
                plan: None,
            })
        }

        fn undo_operation(
            &self,
            _: &BackendRequestContext,
            request: &UndoRequest,
        ) -> Result<UndoResult, BackendError> {
            Ok(UndoResult {
                operation: request.operation.clone(),
                result: ToolResult {
                    call_id: "undo".into(),
                    ok: true,
                    changed: true,
                    data: None,
                    error: None,
                    preview: None,
                },
            })
        }
    }

    struct Revisions {
        value: u64,
    }

    impl RevisionPort for Revisions {
        fn current_revision(
            &self,
            _: &BackendRequestContext,
            path: &str,
        ) -> Result<Option<u64>, BackendError> {
            Ok((path == "note.md").then_some(self.value))
        }
    }

    fn make_runtime<'a>(
        state: &'a State,
        ports: &'a Ports,
        revisions: &'a Revisions,
        generations: &'a OperationGenerationCache,
    ) -> InteractionRuntime<'a> {
        InteractionRuntime::new(state, Some(ports), Some(revisions), generations)
    }

    #[test]
    fn a_proposal_instead_of_the_change_keeps_the_run_going() {
        let state = State::default();
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &Ports, &revisions, &generations);
        let resume = |key: &str, suggestion: Option<&str>| {
            let response = runtime
                .begin_confirmation(
                    &context(),
                    key,
                    &ToolDefinition {
                        name: "create_note".into(),
                        description: "create".into(),
                        input_schema: serde_json::json!({}),
                        scopes: vec![BackendScope::Library],
                        read_only: false,
                        requires_confirmation: true,
                    },
                    &ToolCall { id: format!("operation-{key}"), name: "create_note".into(), arguments: serde_json::json!({"path": "note.md"}), round: 1 },
                    1,
                )
                .expect("waiting response");
            let BackendResponse::Operation { status } = response else { panic!("waiting") };
            let resumed = runtime
                .handle_resume(&ResumeRequest {
                    context: context(),
                    idempotency_key: key.into(),
                    request_id: "request-1".into(),
                    operation: status.operation.clone().expect("operation token"),
                    last_event_sequence: 2,
                    decision: ResumeDecision::Confirmation(ConfirmationDecision {
                        operation_id: format!("operation-{key}"),
                        accepted: false,
                        hunk_ids: Vec::new(),
                        approve_all: false,
                        suggestion: suggestion.map(str::to_string),
                    }),
                })
                .expect("resume");
            let BackendResponse::Resumed { status, .. } = resumed else { panic!("resumed") };
            status.state
        };
        assert_eq!(resume("idem-proposal", Some("creala en Inbox")), OperationState::Running);
        assert_eq!(resume("idem-reject", None), OperationState::Cancelled);
        assert_eq!(resume("idem-blank", Some("  ")), OperationState::Cancelled);
    }

    #[test]
    fn confirmation_waits_without_mutation_and_resumes_against_stored_preview() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let response = runtime
            .begin_confirmation(
                &context(),
                "idem-1",
                &ToolDefinition {
                    name: "create_note".into(),
                    description: "create".into(),
                    input_schema: serde_json::json!({}),
                    scopes: vec![BackendScope::Library],
                    read_only: false,
                    requires_confirmation: true,
                },
                &ToolCall {
                    id: "operation-1".into(),
                    name: "create_note".into(),
                    arguments: serde_json::json!({"path": "note.md"}),
                    round: 1,
                },
                1,
            )
            .expect("waiting response");
        let status = match response {
            BackendResponse::Operation { status } => status,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(status.state, OperationState::WaitingConfirmation);
        let decision = ResumeRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: status.operation.clone().expect("operation token"),
            last_event_sequence: 2,
            decision: ResumeDecision::Confirmation(ConfirmationDecision {
                operation_id: "operation-1".into(),
                accepted: true,
                hunk_ids: vec!["hunk-1".into()],
                approve_all: false,
                suggestion: None,
            }),
        };
        let resumed = runtime.handle_resume(&decision).expect("resume");
        assert!(matches!(
            resumed,
            BackendResponse::Resumed { status, .. }
                if status.state == OperationState::Running
        ));
    }

    #[test]
    fn stale_generation_and_changed_preview_are_rejected() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let token = OperationToken {
            operation_id: "operation-1".into(),
            generation: 1,
        };
        runtime
            .begin_waiting(
                &context(),
                "idem-1",
                OperationReview {
                    operation: token.clone(),
                    state: OperationState::WaitingConfirmation,
                    changed: false,
                    can_undo: false,
                    preview: Some(preview("operation-1")),
                    clarification: None,
                    plan: None,
                },
                1,
            )
            .expect("stored operation");
        let stale = ResumeRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: OperationToken {
                generation: 2,
                ..token.clone()
            },
            last_event_sequence: 2,
            decision: ResumeDecision::Confirmation(ConfirmationDecision {
                operation_id: "operation-1".into(),
                accepted: true,
                hunk_ids: Vec::new(),
                approve_all: false,
                suggestion: None,
            }),
        };
        assert_eq!(
            runtime.handle_resume(&stale).expect_err("stale").code,
            BackendErrorCode::Conflict
        );

        let changed = ResumeRequest {
            operation: token,
            decision: ResumeDecision::Confirmation(ConfirmationDecision {
                operation_id: "operation-1".into(),
                accepted: true,
                hunk_ids: Vec::new(),
                approve_all: false,
                suggestion: None,
            }),
            ..stale
        };
        let changed_revisions = Revisions { value: 5 };
        let changed_runtime = make_runtime(&state, &ports, &changed_revisions, &generations);
        assert_eq!(
            changed_runtime
                .handle_resume(&changed)
                .expect_err("changed")
                .code,
            BackendErrorCode::Conflict
        );
    }

    #[test]
    fn clarification_idempotency_and_cancellation_are_explicit() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let token = runtime.next_operation_token(&context(), "clarification-1");
        runtime
            .begin_clarification(
                &context(),
                "idem-1",
                ClarificationRequest {
                    clarification_id: "clarification-1".into(),
                    operation: token.clone(),
                    question: "Which note?".into(),
                    options: vec![ClarificationOption {
                        id: "one".into(),
                        label: "One".into(),
                    }],
                    allow_free_text: false,
                },
                1,
            )
            .expect("waiting clarification");
        assert!(runtime
            .existing_status_for_request(&context(), "idem-1", 1)
            .expect("idempotency")
            .is_some());
        let request = ResumeRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: token,
            last_event_sequence: 2,
            decision: ResumeDecision::Clarification(ClarificationAnswer {
                clarification_id: "clarification-1".into(),
                operation: OperationToken {
                    operation_id: "clarification-1".into(),
                    generation: 1,
                },
                answer: "One".into(),
                option_id: Some("one".into()),
            }),
        };
        runtime
            .handle_resume(&request)
            .expect("clarification resume");
        let cancel = CancelRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: Some(OperationToken {
                operation_id: "clarification-1".into(),
                generation: 1,
            }),
        };
        assert!(matches!(
            runtime.handle_cancel(&cancel).expect("cancel"),
            BackendResponse::Cancelled { .. }
        ));
    }

    #[test]
    fn plan_resume_review_and_undo_use_explicit_port_paths() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let token = OperationToken {
            operation_id: "operation-1".into(),
            generation: 1,
        };
        runtime
            .begin_plan(
                &context(),
                "idem-plan",
                token.clone(),
                ExecutionPlan {
                    plan_id: "plan-1".into(),
                    generation: 1,
                    title: "Plan".into(),
                    status: PlanStatus::AwaitingApproval,
                    steps: vec![PlanStep {
                        id: "step-1".into(),
                        label: "Read".into(),
                        operation_id: None,
                        status: crate::PlanStepStatus::Pending,
                    }],
                },
                1,
            )
            .expect("plan waiting");
        let resumed = ResumeRequest {
            context: context(),
            idempotency_key: "idem-plan".into(),
            request_id: "request-1".into(),
            operation: token.clone(),
            last_event_sequence: 2,
            decision: ResumeDecision::Plan(PlanDecision {
                plan_id: "plan-1".into(),
                generation: 1,
                accepted: true,
                step_ids: vec!["step-1".into()],
                suggestion: None,
                approve_all: false,
            }),
        };
        assert!(matches!(
            runtime.handle_resume(&resumed).expect("plan resume"),
            BackendResponse::Resumed { status, .. }
                if status.state == OperationState::Running
        ));

        let completed = OperationReview {
            operation: token.clone(),
            state: OperationState::Completed,
            changed: true,
            can_undo: true,
            preview: Some(preview("operation-1")),
            clarification: None,
            plan: None,
        };
        state
            .store_operation(&context(), "idem-done", &completed)
            .expect("completed operation");
        let review = runtime
            .handle_review(&ReviewRequest {
                context: context(),
                idempotency_key: "idem-done".into(),
                request_id: "request-1".into(),
                operation: token.clone(),
            })
            .expect("review");
        assert!(matches!(review, BackendResponse::Review { .. }));
        let undo = runtime
            .handle_undo(&UndoOperationRequest {
                context: context(),
                idempotency_key: "idem-done".into(),
                request_id: "request-1".into(),
                operation: token,
            })
            .expect("undo");
        assert!(matches!(undo, BackendResponse::Undo { .. }));
    }

    fn clarification(runtime: &InteractionRuntime<'_>, id: &str) -> ClarificationRequest {
        ClarificationRequest {
            clarification_id: id.into(),
            operation: runtime.next_operation_token(&context(), id),
            question: "Which note?".into(),
            options: vec![ClarificationOption {
                id: "one".into(),
                label: "One".into(),
            }],
            allow_free_text: false,
        }
    }

    #[test]
    fn a_second_interaction_replaces_a_resumed_checkpoint() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let first = clarification(&runtime, "clarification-1");
        runtime
            .begin_clarification(&context(), "idem-1", first.clone(), 1)
            .expect("first waiting");
        let pending = clarification(&runtime, "clarification-2");
        assert_eq!(
            runtime
                .begin_clarification(&context(), "idem-1", pending, 2)
                .expect_err("only one pending interaction")
                .code,
            BackendErrorCode::Conflict
        );
        let mut resumed = state.review.lock().expect("review").clone().expect("stored");
        resumed.state = OperationState::Running;
        *state.review.lock().expect("review") = Some(resumed);
        let second = clarification(&runtime, "clarification-2");
        let response = runtime
            .begin_clarification(&context(), "idem-1", second.clone(), 3)
            .expect("second waiting");
        let BackendResponse::Operation { status } = response else {
            panic!("expected an operation status");
        };
        assert_eq!(status.operation, Some(second.operation));
        assert_eq!(status.state, OperationState::WaitingClarification);
    }

    #[test]
    fn generations_are_scoped_by_library_and_user() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let first = clarification(&runtime, "call-1");
        runtime
            .begin_clarification(&context(), "idem-1", first, 1)
            .expect("waiting");
        let mut other = context();
        other.library_id = "library-2".into();
        assert_eq!(runtime.next_operation_token(&other, "call-1").generation, 1);
        assert_eq!(runtime.next_operation_token(&context(), "call-1").generation, 2);
    }

    #[test]
    fn cancel_and_status_without_token_target_the_active_run() {
        let state = State::default();
        let ports = Ports;
        let revisions = Revisions { value: 4 };
        let generations = OperationGenerationCache::default();
        let runtime = make_runtime(&state, &ports, &revisions, &generations);
        let control = RequestControl::new(None);
        runtime
            .register_control(&context(), control.clone())
            .expect("register");
        let status = GetOperationRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: None,
            last_event_sequence: 7,
        };
        let BackendResponse::Operation { status: running } =
            runtime.handle_get_operation(&status).expect("running status")
        else {
            panic!("expected an operation status");
        };
        assert_eq!(running.state, OperationState::Running);
        assert_eq!(running.last_event_sequence, 7);
        let cancel = CancelRequest {
            context: context(),
            idempotency_key: "idem-1".into(),
            request_id: "request-1".into(),
            operation: None,
        };
        assert!(matches!(
            runtime.handle_cancel(&cancel).expect("cancel"),
            BackendResponse::Cancelled { .. }
        ));
        assert!(control.is_cancelled());
        assert_eq!(
            runtime.handle_cancel(&cancel).expect_err("nothing left").code,
            BackendErrorCode::NotFound
        );
        assert_eq!(
            runtime.handle_get_operation(&status).expect_err("finished").code,
            BackendErrorCode::NotFound
        );
    }
}
