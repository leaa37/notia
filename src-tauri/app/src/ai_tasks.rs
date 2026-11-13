//! AI provider checks and one-shot AI tasks of the interface.
//!
//! The settings screen checks the provider and lists its models with the
//! preferences being edited; the editor turns handwriting into LaTeX, and
//! Meeting uses [`complete`] and [`stream_complete`] for its live answers and
//! insights. Each operation runs on the transport of the platform (the Rust
//! HTTP client on desktop, the Kotlin AI bridge on Android); the interface
//! only sends the preferences and the input.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use notia_backend_core::ai_settings::{
    self, AiModelOptionDto, AiSettings, AiSettingsInput, CapabilitySource,
};
use notia_backend_core::RequestControl;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::host::{AppHandle, Manager};

use crate::backend::{BackendError, BackendErrorCode};
use crate::backend_ollama::OllamaTransport;
use crate::services::ai_service::{AiChatMessage, AiHttpSettings};

const HEALTH_CACHE_TTL: Duration = Duration::from_secs(10);
const MODEL_LIST_CACHE_TTL: Duration = Duration::from_secs(30);
/// A model's context limit only changes when the model is replaced.
const MODEL_CONTEXT_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const TASK_TIMEOUT: Duration = Duration::from_secs(180);
const INSPECTION_CONCURRENCY: usize = 4;

/// Recent provider answers, so opening settings or a chat does not hit the
/// provider on every render.
#[derive(Default)]
pub(crate) struct AiTasksState {
    health: Mutex<Option<(String, Instant, AiHealthDto)>>,
    models: Mutex<HashMap<String, (Instant, Vec<AiModelOptionDto>)>>,
    /// Context limit of each model, as Ollama reports it.
    contexts: Mutex<HashMap<String, (Instant, Option<usize>)>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiHealthDto {
    ok: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiSettingsPayload {
    settings: AiSettingsInput,
    /// Skip the cached answer (the person asked to check again).
    #[serde(default)]
    fresh: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InkMathPayload {
    settings: AiSettingsInput,
    image_base64: String,
}

fn http(settings: &AiSettings) -> AiHttpSettings {
    AiHttpSettings {
        ollama_url: settings.ollama_url.clone(),
        api_key: settings.api_key.clone(),
    }
}

fn provider_error(message: String) -> BackendError {
    BackendError::new(BackendErrorCode::ProviderUnavailable, message, true)
}

fn blocking_error() -> BackendError {
    BackendError::new(BackendErrorCode::Internal, "La operación de IA se interrumpió.", true)
}

#[cfg(not(target_os = "android"))]
fn provider_health(_app: &AppHandle, settings: &AiSettings) -> Result<AiHealthDto, String> {
    let result = crate::host::async_runtime::block_on(crate::services::ai_service::check_ollama_health(&http(settings)))?;
    Ok(AiHealthDto { ok: result.ok, message: result.message, default_model: result.default_model })
}

#[cfg(target_os = "android")]
fn provider_health(app: &AppHandle, settings: &AiSettings) -> Result<AiHealthDto, String> {
    let raw = crate::mobile_ai_bridge::call_android_ai_plugin(
        app,
        "healthCheck",
        serde_json::json!({ "ollamaUrl": settings.ollama_url, "apiKey": settings.api_key }),
    )?;
    let ok = raw.get("ok").and_then(Value::as_bool).unwrap_or(false);
    Ok(AiHealthDto {
        ok,
        message: raw
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| if ok { "Conexion correcta con Ollama." } else { "No se pudo conectar con la IA." }.to_string()),
        default_model: raw.get("defaultModel").and_then(Value::as_str).map(str::to_string),
    })
}

#[cfg(not(target_os = "android"))]
fn provider_model_names(_app: &AppHandle, settings: &AiSettings) -> Result<Vec<String>, String> {
    crate::host::async_runtime::block_on(crate::services::ai_service::list_ollama_models(&http(settings)))
        .map(|result| result.models)
}

#[cfg(target_os = "android")]
fn provider_model_names(app: &AppHandle, settings: &AiSettings) -> Result<Vec<String>, String> {
    let raw = crate::mobile_ai_bridge::call_android_ai_plugin(
        app,
        "listModels",
        serde_json::json!({ "ollamaUrl": settings.ollama_url, "apiKey": settings.api_key }),
    )?;
    Ok(raw
        .get("models")
        .and_then(Value::as_array)
        .map(|models| models.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default())
}

#[cfg(not(target_os = "android"))]
fn provider_capabilities(_app: &AppHandle, settings: &AiSettings, model: &str) -> Result<Vec<String>, String> {
    crate::host::async_runtime::block_on(crate::services::ai_service::inspect_ollama_model(&http(settings), model))
        .map(|result| result.capabilities)
}

#[cfg(not(target_os = "android"))]
fn provider_context_length(_app: &AppHandle, settings: &AiSettings, model: &str) -> Result<Option<u64>, String> {
    crate::host::async_runtime::block_on(crate::services::ai_service::inspect_ollama_model(&http(settings), model))
        .map(|result| result.context_length)
}

#[cfg(target_os = "android")]
fn provider_context_length(app: &AppHandle, settings: &AiSettings, model: &str) -> Result<Option<u64>, String> {
    let raw = crate::mobile_ai_bridge::call_android_ai_plugin(
        app,
        "inspectModel",
        serde_json::json!({ "ollamaUrl": settings.ollama_url, "apiKey": settings.api_key, "model": model }),
    )?;
    Ok(raw.get("contextLength").and_then(Value::as_u64))
}

/// The model's context limit in tokens, so a task never asks Ollama for
/// more. `None` when the provider does not report one (a cloud model, an
/// unreachable endpoint): the task then asks for what it needs.
fn model_context(app: &AppHandle, settings: &AiSettings, model: &str) -> Option<usize> {
    let state = app.state::<AiTasksState>();
    let key = format!("{}::{model}", models_key(settings));
    if let Ok(guard) = state.contexts.lock() {
        if let Some((at, context)) = guard.get(&key) {
            if at.elapsed() < MODEL_CONTEXT_CACHE_TTL {
                return *context;
            }
        }
    }
    let context = provider_context_length(app, settings, model)
        .ok()
        .flatten()
        .map(|tokens| tokens as usize)
        // A tiny or absurd value is a reporting problem, not a limit.
        .filter(|tokens| *tokens >= 2_048);
    if let Ok(mut guard) = state.contexts.lock() {
        guard.insert(key, (Instant::now(), context));
    }
    context
}

#[cfg(target_os = "android")]
fn provider_capabilities(app: &AppHandle, settings: &AiSettings, model: &str) -> Result<Vec<String>, String> {
    let raw = crate::mobile_ai_bridge::call_android_ai_plugin(
        app,
        "inspectModel",
        serde_json::json!({ "ollamaUrl": settings.ollama_url, "apiKey": settings.api_key, "model": model }),
    )?;
    Ok(raw
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default())
}

/// Desktop Ollama may omit capabilities; the Android bridge reports them all.
#[cfg(not(target_os = "android"))]
const CAPABILITY_SOURCE: CapabilitySource = CapabilitySource::Partial;
#[cfg(target_os = "android")]
const CAPABILITY_SOURCE: CapabilitySource = CapabilitySource::Exact;

fn health_key(settings: &AiSettings) -> String {
    format!(
        "{}::{}::{}",
        settings.ollama_url,
        settings.selected_model,
        if settings.api_key.is_empty() { "missing" } else { "configured" }
    )
}

fn models_key(settings: &AiSettings) -> String {
    format!("{}::{}", settings.ollama_url, if settings.api_key.is_empty() { "missing" } else { "configured" })
}

fn check_health(app: &AppHandle, settings: &AiSettings, fresh: bool) -> AiHealthDto {
    let state = app.state::<AiTasksState>();
    let key = health_key(settings);
    if !fresh {
        if let Ok(guard) = state.health.lock() {
            if let Some((cached_key, at, result)) = guard.as_ref() {
                if *cached_key == key && at.elapsed() < HEALTH_CACHE_TTL {
                    return result.clone();
                }
            }
        }
    }
    let result = provider_health(app, settings).unwrap_or_else(|message| AiHealthDto {
        ok: false,
        message,
        default_model: None,
    });
    if let Ok(mut guard) = state.health.lock() {
        *guard = Some((key, Instant::now(), result.clone()));
    }
    result
}

fn list_models(app: &AppHandle, settings: &AiSettings) -> Result<Vec<AiModelOptionDto>, BackendError> {
    let state = app.state::<AiTasksState>();
    let key = models_key(settings);
    if let Ok(guard) = state.models.lock() {
        if let Some((at, models)) = guard.get(&key) {
            if at.elapsed() < MODEL_LIST_CACHE_TTL {
                return Ok(models.clone());
            }
        }
    }
    let mut names = provider_model_names(app, settings).map_err(provider_error)?;
    names.iter_mut().for_each(|name| *name = name.trim().to_string());
    names.retain(|name| !name.is_empty());
    names.sort();
    names.dedup();
    // Capabilities are inspected a few models at a time.
    let mut models = Vec::with_capacity(names.len());
    for chunk in names.chunks(INSPECTION_CONCURRENCY) {
        let inspected = std::thread::scope(|scope| {
            let handles = chunk
                .iter()
                .map(|name| scope.spawn(move || provider_capabilities(app, settings, name).ok()))
                .collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().ok().flatten()).collect::<Vec<_>>()
        });
        for (name, capabilities) in chunk.iter().zip(inspected) {
            models.push(ai_settings::model_option(name, capabilities.as_deref(), CAPABILITY_SOURCE));
        }
    }
    if let Ok(mut guard) = state.models.lock() {
        guard.insert(key, (Instant::now(), models.clone()));
    }
    Ok(models)
}

fn resolve_model(app: &AppHandle, settings: &AiSettings) -> Result<String, BackendError> {
    if !settings.selected_model.is_empty() {
        return Ok(settings.selected_model.clone());
    }
    let names = list_models(app, settings)?.into_iter().map(|model| model.name).collect::<Vec<_>>();
    ai_settings::resolve_model(settings, &names)
}

fn task_messages(system: &str, user: &str, images: Vec<String>) -> Vec<AiChatMessage> {
    let message = |role: &str, content: &str, images: Vec<String>| AiChatMessage {
        role: role.to_string(),
        content: content.to_string(),
        images,
        tool_calls: Vec::new(),
        tool_name: None,
    };
    vec![message("system", system, Vec::new()), message("user", user, images)]
}

/// One answer from the provider, without tools and without thinking.
pub(crate) fn complete(app: &AppHandle, settings: &AiSettings, system: &str, user: &str, images: Vec<String>) -> Result<String, BackendError> {
    // A task that looks at images (a formula, handwriting) goes to the
    // model chosen for them.
    let model = match settings.separate_image_model() {
        Some(image_model) if !images.is_empty() => image_model.to_string(),
        _ => resolve_model(app, settings)?,
    };
    let options = ai_settings::task_options(
        system.chars().count() + user.chars().count(),
        images.len(),
        model_context(app, settings, &model),
    );
    let answer = crate::backend_ollama::platform_ollama_transport(app).chat(
        &http(settings),
        &model,
        &task_messages(system, user, images),
        &Value::Bool(false),
        &options,
        &RequestControl::new(Some(TASK_TIMEOUT)),
    )?;
    let answer = answer.answer.trim().to_string();
    if answer.is_empty() {
        return Err(provider_error("La IA no devolvio contenido.".to_string()));
    }
    Ok(answer)
}

/// Like [`complete`], hearing the text as it arrives. `control` can cancel it.
pub(crate) fn stream_complete(
    app: &AppHandle,
    settings: &AiSettings,
    system: &str,
    user: &str,
    control: &RequestControl,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, BackendError> {
    let model = resolve_model(app, settings)?;
    let mut text = String::new();
    let answer = crate::backend_ollama::platform_ollama_transport(app).stream_chat(
        &http(settings),
        &model,
        &task_messages(system, user, Vec::new()),
        &Value::Bool(false),
        &ai_settings::task_options(system.chars().count() + user.chars().count(), 0, model_context(app, settings, &model)),
        control,
        &mut |delta| {
            if let crate::services::ai_service::AiChatStreamDelta::Content(delta) = delta {
                text.push_str(&delta);
                on_text(&text);
            }
            Ok(())
        },
    )?;
    let answer = if answer.trim().is_empty() { text } else { answer };
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        return Err(provider_error("La IA no devolvio contenido.".to_string()));
    }
    Ok(answer)
}

/// Timeout of one AI task.
pub(crate) fn task_control() -> RequestControl {
    RequestControl::new(Some(TASK_TIMEOUT))
}

pub(crate) async fn ai_check_health(app: AppHandle, payload: AiSettingsPayload) -> Result<AiHealthDto, BackendError> {
    crate::host::async_runtime::spawn_blocking(move || check_health(&app, &payload.settings.normalize(), payload.fresh))
        .await
        .map_err(|_| blocking_error())
}

pub(crate) async fn ai_list_models(app: AppHandle, payload: AiSettingsPayload) -> Result<Vec<AiModelOptionDto>, BackendError> {
    crate::host::async_runtime::spawn_blocking(move || list_models(&app, &payload.settings.normalize()))
        .await
        .map_err(|_| blocking_error())?
}

/// Model a chat uses with these preferences.
pub(crate) async fn ai_resolve_model(app: AppHandle, payload: AiSettingsPayload) -> Result<String, BackendError> {
    crate::host::async_runtime::spawn_blocking(move || resolve_model(&app, &payload.settings.normalize()))
        .await
        .map_err(|_| blocking_error())?
}

/// Handwritten formula to LaTeX.
pub(crate) async fn ai_recognize_inkmath(app: AppHandle, payload: InkMathPayload) -> Result<String, BackendError> {
    let image = payload.image_base64.trim().to_string();
    if image.is_empty() {
        return Err(BackendError::invalid_input("La imagen de InkMath esta vacia."));
    }
    crate::host::async_runtime::spawn_blocking(move || {
        let answer = complete(
            &app,
            &payload.settings.normalize(),
            ai_settings::INKMATH_SYSTEM_PROMPT,
            ai_settings::INKMATH_USER_PROMPT,
            vec![image],
        )?;
        ai_settings::clean_latex_answer(&answer)
            .ok_or_else(|| provider_error("Ollama no devolvio una formula LaTeX.".to_string()))
    })
    .await
    .map_err(|_| blocking_error())?
}
