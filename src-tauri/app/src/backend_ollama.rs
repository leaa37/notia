//! Ollama adapter for the Tauri host.
//!
//! The backend core only knows `AgentProvider`. This module owns the Ollama
//! configuration and translates that provider contract to the existing native
//! Ollama service without exposing Tauri types to the core.

#[cfg(not(target_os = "android"))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use notia_backend_core::{
    AgentProvider, BackendError, BackendErrorCode, ProviderMessage, ProviderMessageRole,
    ProviderRequest, ProviderResponse, ProviderStreamDelta, ProviderToolCall, RequestControl,
};
use notia_backend_core::tool_call_recovery;
use serde_json::{json, Value};

use crate::services::ai_service::{
    AiChatMessage, AiChatResult, AiChatStreamDelta, AiHttpSettings, AiToolCall, AiToolCallFunction,
    AiWebSearchResponse,
};

const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(20);
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 600;

#[derive(Clone)]
pub struct OllamaProviderConfig {
    settings: AiHttpSettings,
    model: String,
    /// Reads the images for `model`, which then gets only their text.
    image_model: Option<String>,
    think: Value,
    tool_timeout_secs: u64,
}

impl OllamaProviderConfig {
    pub fn new(
        settings: AiHttpSettings,
        model: impl Into<String>,
        think: Value,
    ) -> Result<Self, BackendError> {
        Self::with_tool_timeout(settings, model, think, DEFAULT_TOOL_TIMEOUT_SECS)
    }

    pub fn with_tool_timeout(
        settings: AiHttpSettings,
        model: impl Into<String>,
        think: Value,
        tool_timeout_secs: u64,
    ) -> Result<Self, BackendError> {
        if !(1..=600).contains(&tool_timeout_secs) {
            return Err(BackendError::invalid_input(
                "El tiempo de espera de herramientas debe estar entre 1 y 600 segundos.",
            ));
        }
        Ok(Self {
            settings,
            model: model.into(),
            image_model: None,
            think,
            tool_timeout_secs,
        })
    }

    /// Images go to `image_model` when it is set and differs from the model:
    /// it describes them and the model works with that text.
    pub fn with_image_model(mut self, image_model: &str) -> Self {
        self.image_model =
            notia_backend_core::ai_settings::separate_image_model(&self.model, image_model).map(str::to_string);
        self
    }
}

/// Descriptions the image model already wrote, by model and image: an image
/// stays in the conversation (and the router sees it first) and is read once.
const IMAGE_DESCRIPTIONS_CAPACITY: usize = 256;

#[derive(Default)]
struct ImageDescriptions {
    by_key: HashMap<String, String>,
    order: VecDeque<String>,
}

fn image_descriptions() -> &'static Mutex<ImageDescriptions> {
    static DESCRIPTIONS: OnceLock<Mutex<ImageDescriptions>> = OnceLock::new();
    DESCRIPTIONS.get_or_init(|| Mutex::new(ImageDescriptions::default()))
}

fn image_description_key(image_model: &str, image: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(image_model.as_bytes());
    hasher.update([0]);
    hasher.update(image.as_bytes());
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn remembered_description(key: &str) -> Option<String> {
    image_descriptions().lock().ok()?.by_key.get(key).cloned()
}

fn remember_description(key: String, description: &str) {
    let Ok(mut descriptions) = image_descriptions().lock() else {
        return;
    };
    if descriptions.by_key.insert(key.clone(), description.to_string()).is_none() {
        descriptions.order.push_back(key);
    }
    while descriptions.order.len() > IMAGE_DESCRIPTIONS_CAPACITY {
        if let Some(oldest) = descriptions.order.pop_front() {
            descriptions.by_key.remove(&oldest);
        }
    }
}

/// Port used by the provider adapter. The native implementation delegates to
/// `services::ai_service`; tests can use a deterministic mock at this boundary.
pub trait OllamaTransport: Send + Sync {
    /// `options` are Ollama's model options (`num_ctx`, `num_predict`);
    /// `Value::Null` sends none.
    fn chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
    ) -> Result<AiChatResult, BackendError>;

    fn tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
    ) -> Result<Value, BackendError>;

    fn stream_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<String, BackendError>;

    /// A tool round streamed: content and thinking arrive as deltas and the
    /// result has the shape of `tool_chat` (`{"message": {...}}`). A
    /// transport without it asks the round without streaming.
    #[allow(clippy::too_many_arguments)]
    fn stream_tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<Value, BackendError> {
        let _ = on_delta;
        self.tool_chat(settings, model, messages, tools, think, timeout_secs, control)
    }
}

/// Transport of the current platform. Desktop talks to Ollama over the native
/// Rust HTTP client; Android goes through the Kotlin AI bridge, which owns the
/// platform network stack and foreground-work continuity.
#[cfg(not(target_os = "android"))]
pub type PlatformOllamaTransport = NativeOllamaTransport;
#[cfg(target_os = "android")]
pub type PlatformOllamaTransport = AndroidOllamaTransport;

#[cfg(not(target_os = "android"))]
pub fn platform_ollama_transport(_app: &crate::host::AppHandle) -> PlatformOllamaTransport {
    NativeOllamaTransport
}

#[cfg(target_os = "android")]
pub fn platform_ollama_transport(app: &crate::host::AppHandle) -> PlatformOllamaTransport {
    AndroidOllamaTransport { app: app.clone() }
}

/// Runs an Ollama web search on the platform transport and returns sanitized,
/// bounded results. The query must already be sanitized by the caller.
pub fn search_web(
    app: &crate::host::AppHandle,
    settings: &AiHttpSettings,
    query: &str,
    max_results: u32,
    control: &RequestControl,
) -> Result<AiWebSearchResponse, BackendError> {
    if settings.api_key.trim().is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::ProviderUnavailable,
            "La busqueda web de Ollama requiere una API key configurada.",
            false,
        ));
    }
    if crate::services::ai_service::contains_sensitive_web_query_data(query) {
        return Err(BackendError::invalid_input(
            "La busqueda web fue bloqueada porque la consulta no es publica y segura.",
        ));
    }
    let max_results = max_results.clamp(1, 10);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let settings = settings.clone();
        let query = query.trim().to_string();
        run_with_control(
            move || {
                crate::host::async_runtime::block_on(crate::services::ai_service::search_ollama_web(
                    &settings,
                    &query,
                    max_results,
                ))
            },
            control,
        )
    }
    #[cfg(target_os = "android")]
    {
        let app = app.clone();
        let payload = json!({
            "ollamaUrl": settings.ollama_url,
            "apiKey": settings.api_key,
            "query": query.trim(),
            "maxResults": max_results,
        });
        let raw = run_with_control(
            move || crate::mobile_ai_bridge::call_android_ai_plugin(&app, "webSearch", payload),
            control,
        )?;
        crate::services::ai_service::normalize_web_search_payload(raw, max_results)
            .map_err(map_service_error)
    }
}

#[cfg(not(target_os = "android"))]
pub struct NativeOllamaTransport;

#[cfg(not(target_os = "android"))]
impl OllamaTransport for NativeOllamaTransport {
    fn chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
    ) -> Result<AiChatResult, BackendError> {
        let settings = settings.clone();
        let model = model.to_string();
        let messages = messages.to_vec();
        let think = think.clone();
        let options = options.clone();
        run_with_control(
            move || {
                let result = crate::host::async_runtime::block_on(crate::services::ai_service::run_ollama_chat(
                    &settings, &model, &messages, &think, &options,
                ));
                // An empty answer is not a transport failure: the agent
                // decides how to continue, as with streaming.
                match result {
                    Err(error) if error == EMPTY_ANSWER => Ok(crate::services::ai_service::AiChatResult { answer: String::new() }),
                    other => other,
                }
            },
            control,
        )
    }

    fn tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
    ) -> Result<Value, BackendError> {
        let settings = settings.clone();
        let model = model.to_string();
        let messages = messages.clone();
        let tools = tools.clone();
        let think = think.clone();
        run_with_control(
            move || {
                crate::host::async_runtime::block_on(crate::services::ai_service::run_ollama_tool_chat(
                    &settings,
                    &model,
                    &messages,
                    &tools,
                    &think,
                    timeout_secs,
                ))
            },
            control,
        )
    }

    fn stream_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<String, BackendError> {
        let settings = settings.clone();
        let model = model.to_string();
        let messages = messages.to_vec();
        let think = think.clone();
        let options = options.clone();
        let result = drive_native_stream(control, on_delta, move |cancellation, on_delta| {
            crate::host::async_runtime::block_on(
                crate::services::ai_service::stream_ollama_chat_with_cancellation(
                    &settings,
                    &model,
                    &messages,
                    &think,
                    &options,
                    cancellation,
                    on_delta,
                ),
            )
        })?;
        // An empty answer is not a transport failure: the agent decides how
        // to continue (see `run_agent`).
        let result = match result {
            Err(error) if error == EMPTY_ANSWER => Ok(String::new()),
            other => other,
        };
        result.map_err(map_service_error).and_then(|answer| {
            control.check()?;
            Ok(answer)
        })
    }

    fn stream_tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<Value, BackendError> {
        let settings = settings.clone();
        let model = model.to_string();
        let messages = messages.clone();
        let tools = tools.clone();
        let think = think.clone();
        let result = drive_native_stream(control, on_delta, move |cancellation, on_delta| {
            crate::host::async_runtime::block_on(
                crate::services::ai_service::stream_ollama_tool_chat_with_cancellation(
                    &settings,
                    &model,
                    &messages,
                    &tools,
                    &think,
                    timeout_secs,
                    cancellation,
                    on_delta,
                ),
            )
        })?;
        result.map_err(map_service_error).and_then(|payload| {
            control.check()?;
            Ok(payload)
        })
    }
}

/// Runs a native stream on a worker thread: its deltas reach `on_delta` on
/// the calling thread while the request control is polled, and a cancel or
/// a failing consumer stops the worker's stream.
#[cfg(not(target_os = "android"))]
fn drive_native_stream<T, F>(
    control: &RequestControl,
    on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    work: F,
) -> Result<Result<T, String>, BackendError>
where
    T: Send + 'static,
    F: FnOnce(Arc<AtomicBool>, &mut dyn FnMut(AiChatStreamDelta) -> Result<(), String>) -> Result<T, String>
        + Send
        + 'static,
{
    enum StreamMessage<T> {
        Delta(AiChatStreamDelta),
        Done(Result<T, String>),
    }

    control.check()?;
    let (sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    std::thread::spawn(move || {
        let delta_sender = sender.clone();
        let result = work(worker_cancellation, &mut |delta| {
            delta_sender
                .send(StreamMessage::Delta(delta))
                .map_err(|_| "El consumidor del stream se desconecto.".to_string())
        });
        let _ = sender.send(StreamMessage::Done(result));
    });

    loop {
        match receiver.recv_timeout(STREAM_POLL_INTERVAL) {
            Ok(StreamMessage::Delta(delta)) => {
                if let Err(error) = control.check().and_then(|()| on_delta(delta)) {
                    cancellation.store(true, Ordering::Release);
                    return Err(error);
                }
            }
            Ok(StreamMessage::Done(result)) => return Ok(result),
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = control.check() {
                    cancellation.store(true, Ordering::Release);
                    return Err(error);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(BackendError::new(
                    BackendErrorCode::Internal,
                    "El stream de Ollama se cerro sin una respuesta.",
                    true,
                ));
            }
        }
    }
}

/// Android transport over the Kotlin AI bridge. The bridge call is blocking
/// and not interruptible, so cancellation and timeout are enforced by
/// abandoning the worker result; nothing from an abandoned call is used.
#[cfg(target_os = "android")]
pub struct AndroidOllamaTransport {
    app: crate::host::AppHandle,
}

#[cfg(target_os = "android")]
impl OllamaTransport for AndroidOllamaTransport {
    fn chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
    ) -> Result<AiChatResult, BackendError> {
        let messages = serde_json::to_value(messages).map_err(|_| {
            BackendError::invalid_input("Los mensajes no se pudieron serializar para Ollama.")
        })?;
        let mut payload = Self::tool_chat_payload(settings, model, &messages, &Value::Array(Vec::new()), think, DEFAULT_TOOL_TIMEOUT_SECS);
        payload["options"] = options.clone();
        let response = self.call_tool_chat(payload, control)?;
        if response
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(map_service_error(
                "Ollama no pudo completar la solicitud.".to_string(),
            ));
        }
        // An empty answer is not a transport failure: the agent decides how
        // to continue, as with streaming.
        let answer = response
            .pointer("/message/content")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        Ok(AiChatResult { answer })
    }

    fn tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
    ) -> Result<Value, BackendError> {
        self.call_tool_chat(Self::tool_chat_payload(settings, model, messages, tools, think, timeout_secs), control)
    }

    fn stream_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &[AiChatMessage],
        think: &Value,
        options: &Value,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<String, BackendError> {
        let payload = json!({
            "ollamaUrl": settings.ollama_url,
            "apiKey": settings.api_key,
            "model": model,
            "think": think,
            "options": options,
            "messagesJson": serde_json::to_string(messages).map_err(|_| {
                BackendError::invalid_input("Los mensajes no se pudieron serializar para Ollama.")
            })?,
            "timeoutSeconds": DEFAULT_TOOL_TIMEOUT_SECS,
        });
        let value = self.stream_raw(payload, control, on_delta)?;
        Ok(value
            .get("answer")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string())
    }

    fn stream_tool_chat(
        &self,
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<Value, BackendError> {
        let payload = json!({
            "ollamaUrl": settings.ollama_url,
            "apiKey": settings.api_key,
            "model": model,
            "think": think,
            "messagesJson": messages.to_string(),
            "toolsJson": tools.to_string(),
            "timeoutSeconds": timeout_secs,
        });
        let value = self.stream_raw(payload, control, on_delta)?;
        let message = value.get("message").filter(|message| message.is_object()).cloned().unwrap_or_else(|| {
            json!({ "role": "assistant", "content": value.get("answer").cloned().unwrap_or(Value::Null) })
        });
        Ok(json!({ "message": message }))
    }
}

#[cfg(target_os = "android")]
impl AndroidOllamaTransport {
    fn tool_chat_payload(
        settings: &AiHttpSettings,
        model: &str,
        messages: &Value,
        tools: &Value,
        think: &Value,
        timeout_secs: u64,
    ) -> Value {
        json!({
            "ollamaUrl": settings.ollama_url,
            "apiKey": settings.api_key,
            "model": model,
            "think": think,
            "messagesJson": messages.to_string(),
            "toolsJson": tools.to_string(),
            "timeoutSeconds": timeout_secs,
        })
    }

    fn call_tool_chat(&self, payload: Value, control: &RequestControl) -> Result<Value, BackendError> {
        let app = self.app.clone();
        run_with_control(
            move || crate::mobile_ai_bridge::call_android_ai_plugin(&app, "toolChat", payload),
            control,
        )
    }

    /// Streams a backend-built `/api/chat` request through the Kotlin bridge
    /// and returns its final result once the bridge resolved it.
    fn stream_raw(
        &self,
        payload: Value,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
    ) -> Result<Value, BackendError> {
        use crate::mobile_ai_bridge::AndroidStreamEvent;

        control.check()?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = mpsc::channel();
        let app = self.app.clone();
        let worker_request_id = request_id.clone();
        std::thread::spawn(move || {
            crate::mobile_ai_bridge::stream_android_raw_chat(&app, &worker_request_id, payload, sender);
        });
        let cancel = |error: BackendError| {
            crate::mobile_ai_bridge::cancel_android_raw_chat(&self.app, &request_id);
            Err(error)
        };
        loop {
            match receiver.recv_timeout(STREAM_POLL_INTERVAL) {
                Ok(AndroidStreamEvent::Thinking(delta)) => {
                    if let Err(error) = control.check().and_then(|()| on_delta(AiChatStreamDelta::Thinking(delta))) {
                        return cancel(error);
                    }
                }
                Ok(AndroidStreamEvent::Content(delta)) => {
                    if let Err(error) = control.check().and_then(|()| on_delta(AiChatStreamDelta::Content(delta))) {
                        return cancel(error);
                    }
                }
                Ok(AndroidStreamEvent::Done(result)) => {
                    let value = result.map_err(map_service_error)?;
                    if value.get("ok").and_then(Value::as_bool) == Some(false) {
                        let message = value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Ollama no pudo completar la solicitud.")
                            .to_string();
                        return Err(map_service_error(message));
                    }
                    control.check()?;
                    return Ok(value);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Err(error) = control.check() {
                        return cancel(error);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BackendError::new(
                        BackendErrorCode::Internal,
                        "El stream de Ollama se cerro sin una respuesta.",
                        true,
                    ));
                }
            }
        }
    }
}

pub struct OllamaAgentProvider<T = PlatformOllamaTransport> {
    config: OllamaProviderConfig,
    transport: T,
}

impl<T> OllamaAgentProvider<T> {
    pub fn with_transport(config: OllamaProviderConfig, transport: T) -> Self {
        Self { config, transport }
    }
}

impl<T: OllamaTransport> AgentProvider for OllamaAgentProvider<T> {
    fn chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
    ) -> Result<ProviderResponse, BackendError> {
        control.check()?;
        let messages = translate_messages(&self.readable_messages(&request.messages, control)?);
        let answer = self.transport.chat(
            &self.config.settings,
            &self.config.model,
            &messages,
            &self.config.think,
            &Value::Null,
            control,
        )?;
        control.check()?;
        Ok(ProviderResponse {
            message: ProviderMessage {
                role: ProviderMessageRole::Assistant,
                content: tool_call_recovery::clean_answer_text(&answer.answer),
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_name: None,
            },
        })
    }

    fn stream_chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(ProviderStreamDelta) -> Result<(), BackendError>,
    ) -> Result<ProviderResponse, BackendError> {
        control.check()?;
        if !request.tools.is_empty() {
            return self.stream_tool_round(request, control, on_delta);
        }
        let messages = translate_messages(&self.readable_messages(&request.messages, control)?);
        let answer = self.transport.stream_chat(
            &self.config.settings,
            &self.config.model,
            &messages,
            &self.config.think,
            &Value::Null,
            control,
            &mut |delta| {
                control.check()?;
                on_delta(match delta {
                    AiChatStreamDelta::Thinking(value) => ProviderStreamDelta::Thinking(value),
                    AiChatStreamDelta::Content(value) => ProviderStreamDelta::Content(value),
                })
            },
        )?;
        control.check()?;
        // A streamed answer may carry leaked thinking or a call written as
        // text; an answer that was only markup comes back empty so the agent
        // asks the round again with its tools.
        Ok(ProviderResponse {
            message: ProviderMessage {
                role: ProviderMessageRole::Assistant,
                content: tool_call_recovery::clean_answer_text(&answer),
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_name: None,
            },
        })
    }

    fn tool_chat(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
    ) -> Result<ProviderResponse, BackendError> {
        control.check()?;
        let messages =
            serde_json::to_value(translate_messages(&self.readable_messages(&request.messages, control)?)).map_err(|_| {
                BackendError::invalid_input("Los mensajes no se pudieron serializar para Ollama.")
            })?;
        let tools = translate_tools(&request.tools);
        let response = self.transport.tool_chat(
            &self.config.settings,
            &self.config.model,
            &messages,
            &tools,
            &self.config.think,
            self.config.tool_timeout_secs,
            control,
        )?;
        control.check()?;
        let tool_names = request.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>();
        translate_tool_response(response, &tool_names).map_err(map_service_error)
    }
}

impl<T: OllamaTransport> OllamaAgentProvider<T> {
    /// The messages as the model can read them: with a separate image model,
    /// each image becomes the text that model wrote about it.
    fn readable_messages<'a>(
        &self,
        messages: &'a [ProviderMessage],
        control: &RequestControl,
    ) -> Result<Cow<'a, [ProviderMessage]>, BackendError> {
        let Some(image_model) = self.config.image_model.as_deref() else {
            return Ok(Cow::Borrowed(messages));
        };
        if messages.iter().all(|message| message.images.is_empty()) {
            return Ok(Cow::Borrowed(messages));
        }
        let mut readable = messages.to_vec();
        for message in readable.iter_mut().filter(|message| !message.images.is_empty()) {
            let mut parts = Vec::with_capacity(message.images.len() + 1);
            if !message.content.trim().is_empty() {
                parts.push(message.content.clone());
            }
            for (index, image) in message.images.iter().enumerate() {
                let description = self.describe_image(image_model, image, control)?;
                parts.push(format!(
                    "[Imagen {} de este mensaje, leída por el modelo de imágenes]\n{description}",
                    index + 1
                ));
            }
            message.content = parts.join("\n\n");
            message.images.clear();
        }
        Ok(Cow::Owned(readable))
    }

    /// What `image_model` reads in `image`, once per model and image.
    fn describe_image(&self, image_model: &str, image: &str, control: &RequestControl) -> Result<String, BackendError> {
        let key = image_description_key(image_model, image);
        if let Some(description) = remembered_description(&key) {
            return Ok(description);
        }
        control.check()?;
        let message = |role: &str, content: &str, images: Vec<String>| AiChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            images,
            tool_calls: Vec::new(),
            tool_name: None,
        };
        let messages = [
            message("system", notia_backend_core::ai_settings::IMAGE_READER_PROMPT, Vec::new()),
            message("user", "Describí esta imagen.", vec![image.to_string()]),
        ];
        let answer = self
            .transport
            .chat(&self.config.settings, image_model, &messages, &Value::Bool(false), &Value::Null, control)
            .map_err(|error| {
                BackendError::new(
                    error.code,
                    format!("El modelo de imágenes ({image_model}) no pudo leer la imagen: {}", error.message),
                    error.retryable,
                )
            })?;
        let description = tool_call_recovery::clean_answer_text(&answer.answer).trim().to_string();
        if description.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::ProviderUnavailable,
                format!("El modelo de imágenes ({image_model}) no devolvió qué ve en la imagen."),
                true,
            ));
        }
        remember_description(key, &description);
        Ok(description)
    }

    /// A round with tools, streamed: the agent can keep calling tools after
    /// its first result instead of only being able to answer, and the text
    /// the model writes before its calls reaches the person as it arrives.
    fn stream_tool_round(
        &self,
        request: &ProviderRequest,
        control: &RequestControl,
        on_delta: &mut dyn FnMut(ProviderStreamDelta) -> Result<(), BackendError>,
    ) -> Result<ProviderResponse, BackendError> {
        let messages =
            serde_json::to_value(translate_messages(&self.readable_messages(&request.messages, control)?)).map_err(|_| {
                BackendError::invalid_input("Los mensajes no se pudieron serializar para Ollama.")
            })?;
        let tools = translate_tools(&request.tools);
        let response = self.transport.stream_tool_chat(
            &self.config.settings,
            &self.config.model,
            &messages,
            &tools,
            &self.config.think,
            self.config.tool_timeout_secs,
            control,
            &mut |delta| {
                control.check()?;
                on_delta(match delta {
                    AiChatStreamDelta::Thinking(value) => ProviderStreamDelta::Thinking(value),
                    AiChatStreamDelta::Content(value) => ProviderStreamDelta::Content(value),
                })
            },
        )?;
        control.check()?;
        let tool_names = request.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>();
        translate_tool_response(response, &tool_names).map_err(map_service_error)
    }
}

fn translate_messages(messages: &[ProviderMessage]) -> Vec<AiChatMessage> {
    messages
        .iter()
        .map(|message| AiChatMessage {
            role: match message.role {
                ProviderMessageRole::System => "system",
                ProviderMessageRole::User => "user",
                ProviderMessageRole::Assistant => "assistant",
                ProviderMessageRole::Tool => "tool",
            }
            .to_string(),
            content: message.content.clone(),
            // Vision attachments are part of the backend protocol. Do not
            // silently drop them when crossing the Ollama adapter boundary.
            images: message.images.clone(),
            tool_calls: message
                .tool_calls
                .iter()
                .map(|call| AiToolCall {
                    function: AiToolCallFunction {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                })
                .collect(),
            tool_name: message.tool_name.clone(),
        })
        .collect()
}

fn translate_tools(tools: &[notia_backend_core::ToolDefinition]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect(),
    )
}

/// Structured response of a tool round. When the server returned no
/// `tool_calls` but the model wrote them as text (or inside its thinking),
/// the calls to `tool_names` are recovered and the markup removed.
fn translate_tool_response(payload: Value, tool_names: &[&str]) -> Result<ProviderResponse, String> {
    if payload
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err("Ollama devolvio un error al procesar la solicitud.".to_string());
    }

    let message = payload
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "La respuesta de herramientas de Ollama no es valida.".to_string())?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    let function =
                        call.get("function")
                            .and_then(Value::as_object)
                            .ok_or_else(|| {
                                "La respuesta de herramientas de Ollama no es valida.".to_string()
                            })?;
                    let name = function
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            "La respuesta de herramientas de Ollama no es valida.".to_string()
                        })?;
                    Ok(ProviderToolCall {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("ollama-call-{}", index + 1)),
                        name: name.to_string(),
                        arguments: function
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();

    let (content, tool_calls) = if tool_calls.is_empty() {
        let thinking = message.get("thinking").and_then(Value::as_str).unwrap_or_default();
        let recovered = tool_call_recovery::recover_tool_calls(&format!("{thinking}\n{content}"), tool_names)
            .into_iter()
            .enumerate()
            .map(|(index, call)| ProviderToolCall {
                id: format!("recovered-call-{}", index + 1),
                name: call.name,
                arguments: call.arguments,
            })
            .collect::<Vec<_>>();
        (tool_call_recovery::clean_answer_text(&content), recovered)
    } else {
        // The text next to the calls is the agent's note: reasoning leaked
        // into it (up to a stray `</think>`) is not part of it.
        (tool_call_recovery::clean_answer_text(&content), tool_calls)
    };

    Ok(ProviderResponse {
        message: ProviderMessage {
            role: ProviderMessageRole::Assistant,
            content,
            images: Vec::new(),
            tool_calls,
            tool_name: None,
        },
    })
}

/// Runs blocking provider work on a worker thread while polling the request
/// control, so cancellation and the request deadline are honoured even when
/// the underlying call cannot be interrupted.
fn run_with_control<T, F>(work: F, control: &RequestControl) -> Result<T, BackendError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    control.check()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });

    loop {
        match receiver.recv_timeout(STREAM_POLL_INTERVAL) {
            Ok(result) => return result.map_err(map_service_error),
            Err(RecvTimeoutError::Timeout) => control.check()?,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(BackendError::new(
                    BackendErrorCode::Internal,
                    "La llamada a Ollama se cerro sin una respuesta.",
                    true,
                ));
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
const EMPTY_ANSWER: &str = "La IA no devolvio contenido.";

/// Short provider detail safe to show: the `error` field of an Ollama JSON
/// body or a one-line message, without credentials or URLs.
fn provider_detail(error: &str) -> Option<String> {
    let detail = serde_json::from_str::<Value>(error)
        .ok()
        .and_then(|value| value.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| error.to_string());
    let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = detail.to_lowercase();
    let unsafe_detail = ["http", "bearer", "key", "token", "secret", "password"]
        .iter()
        .any(|marker| lower.contains(marker));
    (!detail.is_empty() && !unsafe_detail).then(|| detail.chars().take(160).collect())
}

fn map_service_error(error: String) -> BackendError {
    let normalized = error.to_lowercase();
    if normalized.contains("cancel") || normalized.contains("cancelad") {
        return BackendError::cancelled();
    }
    if normalized.contains("tiempo de espera") || normalized.contains("timeout") {
        return BackendError::timeout();
    }
    if normalized.contains("url de ollama")
        || normalized.contains("modelo de ollama")
        || normalized.contains("no hay mensajes")
        || normalized.contains("no hay herramientas")
        || normalized.contains("tiempo de espera de herramientas")
    {
        return BackendError::invalid_input(safe_validation_message(&normalized));
    }
    if normalized.contains("unauthorized") {
        return BackendError::invalid_input(
            "Ollama rechazó la credencial. Revisá la API key en Configuración → IA.",
        );
    }
    if normalized.contains("not found") && normalized.contains("model") {
        return BackendError::invalid_input(
            "El modelo seleccionado no existe en el servidor de Ollama configurado.",
        );
    }
    if normalized.contains("no se pudo conectar")
        || normalized.contains("se interrumpio el stream")
        || normalized.contains("consumidor del stream")
    {
        return BackendError::new(
            BackendErrorCode::ProviderUnavailable,
            "No se pudo conectar con Ollama.",
            true,
        );
    }
    BackendError::new(
        BackendErrorCode::ProviderUnavailable,
        match provider_detail(&error) {
            Some(detail) => format!("Ollama no pudo completar la solicitud: {detail}"),
            None => "Ollama no pudo completar la solicitud.".to_string(),
        },
        is_server_failure(&normalized),
    )
}

/// A failure of the server itself (an HTTP 5xx, an overloaded model) that
/// usually passes: the round is asked again before ending the turn.
fn is_server_failure(normalized_error: &str) -> bool {
    [
        "internal server error",
        "bad gateway",
        "service unavailable",
        "gateway timeout",
        "overloaded",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
    ]
    .iter()
    .any(|marker| normalized_error.contains(marker))
}

fn safe_validation_message(normalized_error: &str) -> &'static str {
    if normalized_error.contains("url de ollama es obligatoria") {
        "La URL de Ollama es obligatoria."
    } else if normalized_error.contains("url de ollama no es valida") {
        "La URL de Ollama no es valida."
    } else if normalized_error.contains("url de ollama debe usar") {
        "La URL de Ollama debe usar http o https."
    } else if normalized_error.contains("credenciales embebidas") {
        "La URL de Ollama no puede incluir credenciales embebidas."
    } else if normalized_error.contains("modelo de ollama") {
        "El modelo de Ollama es obligatorio."
    } else if normalized_error.contains("no hay herramientas") {
        "No hay herramientas para enviar a la IA."
    } else if normalized_error.contains("tiempo de espera de herramientas") {
        "El tiempo de espera de herramientas no es valido."
    } else {
        "Los datos enviados a Ollama no son validos."
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use notia_backend_core::{
        BackendActor, BackendChannel, BackendRequestContext, BackendScope, ProviderMessage,
        ProviderMessageRole, ToolDefinition,
    };

    const TOOL_RESPONSE_FIXTURE: &str = include_str!("fixtures/ollama-tool-response.json");

    struct MockTransport {
        seen_messages: Arc<Mutex<Vec<AiChatMessage>>>,
        seen_tools: Arc<Mutex<Option<Value>>>,
        response: Value,
    }

    impl OllamaTransport for MockTransport {
        fn chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            messages: &[AiChatMessage],
            _: &Value,
            _: &Value,
            _: &RequestControl,
        ) -> Result<AiChatResult, BackendError> {
            *self.seen_messages.lock().expect("messages lock") = messages.to_vec();
            Ok(AiChatResult {
                answer: "respuesta mock".to_string(),
            })
        }

        fn tool_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            messages: &Value,
            tools: &Value,
            _: &Value,
            _: u64,
            _: &RequestControl,
        ) -> Result<Value, BackendError> {
            *self.seen_messages.lock().expect("messages lock") =
                serde_json::from_value(messages.clone()).expect("messages fixture");
            *self.seen_tools.lock().expect("tools lock") = Some(tools.clone());
            Ok(self.response.clone())
        }

        fn stream_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            messages: &[AiChatMessage],
            _: &Value,
            _: &Value,
            _: &RequestControl,
            on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
        ) -> Result<String, BackendError> {
            *self.seen_messages.lock().expect("messages lock") = messages.to_vec();
            on_delta(AiChatStreamDelta::Thinking("pensando".to_string()))?;
            on_delta(AiChatStreamDelta::Content("respuesta".to_string()))?;
            Ok("respuesta".to_string())
        }
    }

    /// Streams a tool round: a note, then a call.
    struct StreamingToolTransport {
        seen_tools: Arc<Mutex<Option<Value>>>,
    }

    impl OllamaTransport for StreamingToolTransport {
        fn chat(&self, _: &AiHttpSettings, _: &str, _: &[AiChatMessage], _: &Value, _: &Value, _: &RequestControl) -> Result<AiChatResult, BackendError> {
            unreachable!()
        }

        fn tool_chat(&self, _: &AiHttpSettings, _: &str, _: &Value, _: &Value, _: &Value, _: u64, _: &RequestControl) -> Result<Value, BackendError> {
            unreachable!()
        }

        fn stream_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            _: &[AiChatMessage],
            _: &Value,
            _: &Value,
            _: &RequestControl,
            _: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
        ) -> Result<String, BackendError> {
            unreachable!("a round with tools streams with its tools")
        }

        fn stream_tool_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            _: &Value,
            tools: &Value,
            _: &Value,
            _: u64,
            _: &RequestControl,
            on_delta: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
        ) -> Result<Value, BackendError> {
            *self.seen_tools.lock().expect("tools lock") = Some(tools.clone());
            on_delta(AiChatStreamDelta::Content("Voy a leer la nota.".to_string()))?;
            Ok(json!({"message": {
                "role": "assistant",
                "content": "Voy a leer la nota.",
                "tool_calls": [{"function": {"name": "read_note", "arguments": {"path": "a.md"}}}]
            }}))
        }
    }

    fn config() -> OllamaProviderConfig {
        OllamaProviderConfig::new(
            AiHttpSettings {
                ollama_url: "https://ollama.test".to_string(),
                api_key: "secret-fixture-key".to_string(),
            },
            "qwen3:test",
            json!("medium"),
        )
        .expect("valid config")
    }

    fn request(with_tools: bool) -> ProviderRequest {
        ProviderRequest {
            context: BackendRequestContext {
                request_id: "request-1".to_string(),
                library_id: "library-1".to_string(),
                actor: BackendActor {
                    library_user_id: "user-owner".to_string(),
                    external_identity: None,
                },
                channel: BackendChannel::App,
                scope: BackendScope::Library,
                persistence_policy: notia_backend_core::PersistencePolicy::Persistent,
            },
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                content: "lee la nota".to_string(),
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_name: None,
            }],
            tools: if with_tools {
                vec![ToolDefinition {
                    name: "read_note".to_string(),
                    description: "Lee una nota".to_string(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {"path": {"type": "string"}}
                    }),
                    scopes: vec![BackendScope::Library],
                    read_only: true,
                    requires_confirmation: false,
                }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn a_streamed_round_with_tools_sends_them_and_returns_the_calls() {
        let seen_tools = Arc::new(Mutex::new(None));
        let provider = OllamaAgentProvider::with_transport(
            config(),
            StreamingToolTransport { seen_tools: Arc::clone(&seen_tools) },
        );
        let mut deltas = Vec::new();
        let response = provider
            .stream_chat(&request(true), &RequestControl::new(None), &mut |delta| {
                deltas.push(delta);
                Ok(())
            })
            .expect("streamed tool round");
        assert_eq!(deltas, vec![ProviderStreamDelta::Content("Voy a leer la nota.".to_string())]);
        assert_eq!(response.message.content, "Voy a leer la nota.");
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].name, "read_note");
        assert_eq!(response.message.tool_calls[0].arguments, json!({"path": "a.md"}));
        let tools = seen_tools.lock().expect("tools lock").clone().expect("tools sent");
        assert_eq!(tools[0]["function"]["name"], "read_note");
    }

    #[test]
    fn translates_messages_and_tools_without_calling_real_ollama() {
        let seen_messages = Arc::new(Mutex::new(Vec::new()));
        let seen_tools = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_messages: Arc::clone(&seen_messages),
            seen_tools: Arc::clone(&seen_tools),
            response: serde_json::from_str(TOOL_RESPONSE_FIXTURE).expect("tool fixture"),
        };
        let provider = OllamaAgentProvider::with_transport(config(), transport);
        let response = provider
            .tool_chat(&request(true), &RequestControl::new(None))
            .expect("mock tool response");

        assert_eq!(seen_messages.lock().expect("messages lock")[0].role, "user");
        assert_eq!(
            seen_messages.lock().expect("messages lock")[0].content,
            "lee la nota"
        );
        assert_eq!(
            seen_tools
                .lock()
                .expect("tools lock")
                .as_ref()
                .expect("tools")[0]["type"],
            "function"
        );
        assert_eq!(response.message.tool_calls[0].id, "ollama-call-1");
        assert_eq!(response.message.tool_calls[0].name, "read_note");
        assert_eq!(
            response.message.tool_calls[0].arguments["path"],
            "notes/example.md"
        );
    }

    #[test]
    fn translates_stream_deltas_and_final_content() {
        let transport = MockTransport {
            seen_messages: Arc::new(Mutex::new(Vec::new())),
            seen_tools: Arc::new(Mutex::new(None)),
            response: Value::Null,
        };
        let provider = OllamaAgentProvider::with_transport(config(), transport);
        let mut deltas = Vec::new();
        let response = provider
            .stream_chat(&request(false), &RequestControl::new(None), &mut |delta| {
                deltas.push(delta);
                Ok(())
            })
            .expect("mock stream response");

        assert_eq!(response.message.content, "respuesta");
        assert_eq!(
            deltas,
            vec![
                ProviderStreamDelta::Thinking("pensando".to_string()),
                ProviderStreamDelta::Content("respuesta".to_string()),
            ]
        );
    }

    #[test]
    fn recovers_tool_calls_written_as_text_by_local_models() {
        let response = translate_tool_response(
            json!({ "message": {
                "content": "Busco.\n<function=search_library><parameter=query>ticket</parameter></function>",
                "thinking": ""
            }}),
            &["search_library"],
        )
        .expect("response");
        assert_eq!(response.message.content, "Busco.");
        assert_eq!(response.message.tool_calls[0].name, "search_library");
        assert_eq!(response.message.tool_calls[0].arguments, json!({ "query": "ticket" }));

        let untouched = translate_tool_response(
            json!({ "message": { "content": "<function=other><parameter=a>1</parameter></function>" } }),
            &["search_library"],
        )
        .expect("response");
        assert!(untouched.message.tool_calls.is_empty());
    }

    #[test]
    fn the_note_next_to_tool_calls_drops_leaked_reasoning() {
        let response = translate_tool_response(
            json!({ "message": {
                "content": "El usuario quiere cargar la cena. Llamo a log_meal.</think>",
                "tool_calls": [{ "function": { "name": "log_meal", "arguments": { "name": "Puchero" } } }]
            }}),
            &["log_meal"],
        )
        .expect("response");
        assert_eq!(response.message.content, "");
        assert_eq!(response.message.tool_calls[0].name, "log_meal");
    }

    #[test]
    fn checks_control_before_transport_and_maps_provider_errors_structurally() {
        let transport = MockTransport {
            seen_messages: Arc::new(Mutex::new(Vec::new())),
            seen_tools: Arc::new(Mutex::new(None)),
            response: Value::Null,
        };
        let provider = OllamaAgentProvider::with_transport(config(), transport);
        let control = RequestControl::new(None);
        control.cancel();
        assert_eq!(
            provider
                .chat(&request(false), &control)
                .expect_err("cancelled request")
                .code,
            BackendErrorCode::Cancelled
        );
        assert_eq!(
            map_service_error("No se pudo conectar con la IA. api_key=secret".to_string()).code,
            BackendErrorCode::ProviderUnavailable
        );
        assert_eq!(
            map_service_error("{\"error\":\"unauthorized\"}".to_string()).code,
            BackendErrorCode::InvalidInput
        );
        assert_eq!(
            map_service_error("{\"error\":\"model 'x' not found\"}".to_string()).code,
            BackendErrorCode::InvalidInput
        );
        assert!(map_service_error("{\"error\":\"context length exceeded\"}".to_string())
            .message
            .ends_with("context length exceeded"));
        // A server failure is asked again; a rejected request is not.
        assert!(map_service_error("{\"error\":\"Internal Server Error (ref: 6b8fcb4a)\"}".to_string()).retryable);
        assert!(map_service_error("La IA respondio con HTTP 503 Service Unavailable.".to_string()).retryable);
        assert!(!map_service_error("{\"error\":\"context length exceeded\"}".to_string()).retryable);
        assert!(!map_service_error("bad Bearer abc".to_string()).message.contains("abc"));
        assert!(
            !map_service_error("No se pudo conectar con la IA. api_key=secret".to_string())
                .message
                .contains("secret")
        );
    }

    /// Answers as the image model or as the main model, and records each
    /// chat call with its model and messages.
    struct ImageReaderTransport {
        calls: Arc<Mutex<Vec<(String, Vec<AiChatMessage>)>>>,
        reader_fails: bool,
    }

    impl OllamaTransport for ImageReaderTransport {
        fn chat(
            &self,
            _: &AiHttpSettings,
            model: &str,
            messages: &[AiChatMessage],
            _: &Value,
            _: &Value,
            _: &RequestControl,
        ) -> Result<AiChatResult, BackendError> {
            self.calls.lock().expect("calls lock").push((model.to_string(), messages.to_vec()));
            if model == "gemma3:vision" {
                if self.reader_fails {
                    return Err(BackendError::new(BackendErrorCode::ProviderUnavailable, "modelo no encontrado", true));
                }
                return Ok(AiChatResult { answer: "Ticket de almacén: yerba 1 kg $3.500, total $3.500.".to_string() });
            }
            Ok(AiChatResult { answer: "Listo.".to_string() })
        }

        fn tool_chat(&self, _: &AiHttpSettings, _: &str, _: &Value, _: &Value, _: &Value, _: u64, _: &RequestControl) -> Result<Value, BackendError> {
            unreachable!()
        }

        fn stream_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            _: &[AiChatMessage],
            _: &Value,
            _: &Value,
            _: &RequestControl,
            _: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
        ) -> Result<String, BackendError> {
            unreachable!()
        }

        fn stream_tool_chat(
            &self,
            _: &AiHttpSettings,
            _: &str,
            _: &Value,
            _: &Value,
            _: &Value,
            _: u64,
            _: &RequestControl,
            _: &mut dyn FnMut(AiChatStreamDelta) -> Result<(), BackendError>,
        ) -> Result<Value, BackendError> {
            unreachable!()
        }
    }

    /// A request whose message carries one image, unique to the test (the
    /// read images are remembered for the whole process).
    fn request_with_image() -> (ProviderRequest, String) {
        let image = format!("aW1hZ2Vu{}", uuid::Uuid::new_v4().simple());
        let mut request = request(false);
        request.messages[0].content = "cargá este ticket".to_string();
        request.messages[0].images = vec![image.clone()];
        (request, image)
    }

    #[test]
    fn a_separate_image_model_reads_the_images_and_the_model_gets_their_text() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = OllamaAgentProvider::with_transport(
            config().with_image_model(" gemma3:vision "),
            ImageReaderTransport { calls: Arc::clone(&calls), reader_fails: false },
        );
        let (request, image) = request_with_image();
        provider.chat(&request, &RequestControl::new(None)).expect("answer");
        {
            let calls = calls.lock().expect("calls lock");
            assert_eq!(calls.len(), 2);
            let (reader, read) = &calls[0];
            assert_eq!(reader, "gemma3:vision");
            assert_eq!(read[0].content, notia_backend_core::ai_settings::IMAGE_READER_PROMPT);
            assert_eq!(read[1].images, vec![image.clone()]);
            let (model, sent) = &calls[1];
            assert_eq!(model, "qwen3:test");
            assert!(sent.iter().all(|message| message.images.is_empty()));
            assert!(sent[0].content.starts_with("cargá este ticket\n\n[Imagen 1 de este mensaje, leída por el modelo de imágenes]"));
            assert!(sent[0].content.contains("total $3.500"));
        }
        // The same image in a later call is not read again.
        provider.chat(&request, &RequestControl::new(None)).expect("answer");
        let calls = calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].0, "qwen3:test");
    }

    #[test]
    fn without_a_separate_image_model_the_images_go_to_the_model() {
        for image_model in ["", "qwen3:test"] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let provider = OllamaAgentProvider::with_transport(
                config().with_image_model(image_model),
                ImageReaderTransport { calls: Arc::clone(&calls), reader_fails: false },
            );
            let (request, image) = request_with_image();
            provider.chat(&request, &RequestControl::new(None)).expect("answer");
            let calls = calls.lock().expect("calls lock");
            assert_eq!(calls.len(), 1, "{image_model}");
            assert_eq!(calls[0].0, "qwen3:test");
            assert_eq!(calls[0].1[0].images, vec![image]);
            assert_eq!(calls[0].1[0].content, "cargá este ticket");
        }
    }

    #[test]
    fn an_image_the_image_model_cannot_read_stops_the_call_with_its_name() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = OllamaAgentProvider::with_transport(
            config().with_image_model("gemma3:vision"),
            ImageReaderTransport { calls: Arc::clone(&calls), reader_fails: true },
        );
        let (request, _) = request_with_image();
        let error = provider.chat(&request, &RequestControl::new(None)).expect_err("unread image");
        assert!(error.message.contains("El modelo de imágenes (gemma3:vision)"), "{}", error.message);
        assert_eq!(calls.lock().expect("calls lock").len(), 1, "the main model is not called");
    }
}
