use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiHttpSettings {
    pub ollama_url: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "tool_calls")]
    pub tool_calls: Vec<AiToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "tool_name")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiToolCall {
    pub function: AiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[cfg(test)]
mod message_tests {
    use super::{AiChatMessage, AiToolCall, AiToolCallFunction};

    #[test]
    fn preserves_tool_call_metadata_when_messages_cross_the_native_boundary() {
        let message = AiChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            images: Vec::new(),
            tool_calls: vec![AiToolCall {
                function: AiToolCallFunction {
                    name: "read_task_tickets".to_string(),
                    arguments: serde_json::json!({ "ticketIds": ["doc-20"] }),
                },
            }],
            tool_name: None,
        };

        let serialized = serde_json::to_value(&message).expect("message serializes");
        assert_eq!(
            serialized["tool_calls"][0]["function"]["name"],
            "read_task_tickets"
        );
        assert_eq!(
            serialized["tool_calls"][0]["function"]["arguments"]["ticketIds"][0],
            "doc-20"
        );
        let parsed: AiChatMessage = serde_json::from_value(serialized).expect("message parses");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].function.name, "read_task_tickets");
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiHealthResult {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatResult {
    pub answer: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelListResult {
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelDetailsResult {
    pub capabilities: Vec<String>,
    /// The model's context limit in tokens, when Ollama reports it.
    pub context_length: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiWebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source_name: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiWebSearchResponse {
    pub results: Vec<AiWebSearchResult>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use futures::StreamExt;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use reqwest::{Client, Url};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::time::Duration;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const HEALTH_TIMEOUT_SECS: u64 = 15;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const CHAT_TIMEOUT_SECS: u64 = 180;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const WEB_SEARCH_TIMEOUT_SECS: u64 = 30;
const MAX_WEB_SEARCH_QUERY_CHARS: usize = 240;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelDescriptor>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaModelDescriptor {
    name: Option<String>,
    model: Option<String>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
    /// Architecture-prefixed details, such as `qwen35.context_length`.
    #[serde(default)]
    model_info: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OllamaWebSearchResponse {
    #[serde(default)]
    results: Vec<OllamaWebSearchItem>,
}

#[derive(Debug, Deserialize)]
struct OllamaWebSearchItem {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OllamaChatRequest<'a> {
    model: &'a str,
    stream: bool,
    think: &'a serde_json::Value,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    options: &'a serde_json::Value,
    messages: &'a [AiChatMessage],
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaChatResponseMessage>,
    error: Option<String>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaChatResponseMessage {
    content: Option<String>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaChatStreamChunk {
    message: Option<OllamaChatStreamMessage>,
    error: Option<String>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Deserialize)]
struct OllamaChatStreamMessage {
    content: Option<String>,
    thinking: Option<String>,
}

pub enum AiChatStreamDelta {
    Thinking(String),
    Content(String),
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn build_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|error| format!("No se pudo inicializar el cliente HTTP de IA: {error}"))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn build_endpoint(base_url: &str, path: &str) -> Result<Url, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("La URL de Ollama es obligatoria.".to_string());
    }

    let mut url = Url::parse(trimmed).map_err(|_| "La URL de Ollama no es valida.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("La URL de Ollama debe usar http o https.".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("La URL de Ollama no puede incluir credenciales embebidas.".to_string());
    }

    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn build_web_search_endpoint(base_url: &str) -> Result<Url, String> {
    let endpoint = build_endpoint(base_url, "/api/web_search")?;
    let is_ollama_cloud = matches!(endpoint.host_str(), Some("ollama.com" | "www.ollama.com"))
        && endpoint.port().is_none();
    if !is_ollama_cloud {
        return Err("La busqueda web de Ollama requiere Ollama Cloud.".to_string());
    }
    Ok(endpoint)
}

pub(crate) fn contains_sensitive_web_query_data(query: &str) -> bool {
    let normalized = query.trim();
    if normalized.is_empty()
        || normalized.chars().count() > MAX_WEB_SEARCH_QUERY_CHARS
        || normalized.chars().any(char::is_control)
        || normalized.contains('%')
    {
        return true;
    }
    let lower = normalized.to_lowercase();
    let sensitive_markers = [
        "bearer ",
        "authorization:",
        "api_key=",
        "api-key=",
        "api key:",
        "api key=",
        "access_token=",
        "access token:",
        "access token=",
        "password:",
        "password=",
        "secret:",
        "secret=",
        "cookie:",
        "cookie=",
        "password=",
        "-----begin ",
        "sk-",
        "ghp_",
        "gho_",
        "xoxb-",
        "akia",
        "\"password\"",
        "\"secret\"",
        "mi nombre es ",
        "me llamo ",
        "mi correo ",
        "mi email ",
        "mi trabajo ",
        "mi sueldo ",
        "expediente legal",
        "calendario privado",
        "mi telefono ",
        "mi teléfono ",
        "mi domicilio ",
        "mi direccion ",
        "mi dirección ",
        "historia clinica",
        "historia clínica",
        "tarjeta de credito",
        "tarjeta de crédito",
        "cuenta bancaria",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }
    if lower.contains("@")
        || lower.contains("/users/")
        || lower.contains("\\users\\")
        || lower.contains("/home/")
        || lower.contains("\\documents\\")
    {
        return true;
    }
    if normalized.split_whitespace().any(|token| {
        token
            .trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
            .parse::<std::net::Ipv4Addr>()
            .map(|address| address.is_private())
            .unwrap_or(false)
    }) {
        return true;
    }
    let digit_count = normalized.chars().filter(char::is_ascii_digit).count();
    digit_count >= 9 && normalized.chars().any(char::is_whitespace)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn search_ollama_web(
    settings: &AiHttpSettings,
    query: &str,
    max_results: u32,
) -> Result<AiWebSearchResponse, String> {
    if settings.api_key.trim().is_empty() {
        return Err("La busqueda web de Ollama requiere una API key configurada.".to_string());
    }
    if contains_sensitive_web_query_data(query) {
        return Err(
            "La busqueda web fue bloqueada porque la consulta no es publica y segura.".to_string(),
        );
    }
    let normalized_query = query.trim();
    let bounded_results = max_results.clamp(1, 10);
    let client = build_client(WEB_SEARCH_TIMEOUT_SECS)?;
    let endpoint = build_web_search_endpoint(&settings.ollama_url)?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&serde_json::json!({
                "query": normalized_query,
                "max_results": bounded_results,
            })),
        settings,
    )
    .send()
    .await
    .map_err(|error| {
        describe_request_error(error, "No se pudo completar la busqueda web de Ollama.")
    })?;

    if !response.status().is_success() {
        return Err("Ollama no pudo completar la busqueda web.".to_string());
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            describe_request_error(
                error,
                "La respuesta de busqueda web no se pudo interpretar.",
            )
        })?;
    normalize_web_search_payload(payload, bounded_results)
}

/// Sanitizes a raw Ollama `/api/web_search` payload. Shared by the desktop
/// HTTP client and the Android bridge so both platforms expose the same
/// bounded, credential-free results to the agent.
pub(crate) fn normalize_web_search_payload(
    payload: serde_json::Value,
    max_results: u32,
) -> Result<AiWebSearchResponse, String> {
    let payload = serde_json::from_value::<OllamaWebSearchResponse>(payload)
        .map_err(|_| "La respuesta de busqueda web no se pudo interpretar.".to_string())?;
    let results = payload
        .results
        .into_iter()
        .filter_map(|item| {
            let title = sanitize_web_text(item.title?.trim(), 300);
            let raw_url = item.url?.trim().to_string();
            let snippet = sanitize_web_text(
                item.content.unwrap_or_default().trim(),
                2_000,
            );
            let mut parsed_url = reqwest::Url::parse(&raw_url).ok()?;
            if title.is_empty()
                || snippet.is_empty()
                || !matches!(parsed_url.scheme(), "http" | "https")
                || !parsed_url.username().is_empty()
                || parsed_url.password().is_some()
            {
                return None;
            }
            for key in parsed_url.query_pairs().map(|(key, _)| key.to_string()).collect::<Vec<_>>() {
                if matches!(key.to_ascii_lowercase().as_str(), "api_key" | "apikey" | "access_token" | "token" | "secret" | "password" | "cookie" | "authorization") {
                    parsed_url.set_query(None);
                    break;
                }
            }
            parsed_url.set_fragment(None);
            let url = parsed_url.to_string();
            let source_name = parsed_url.host_str()?.to_string();
            Some(AiWebSearchResult {
                title,
                url,
                snippet,
                source_name,
                published_at: None,
            })
        })
        .take(max_results.clamp(1, 10) as usize)
        .collect();

    Ok(AiWebSearchResponse { results })
}

fn sanitize_web_text(value: &str, limit: usize) -> String {
    let without_markup = value
        .replace("<", " ")
        .replace(">", " ")
        .replace("ignore previous instructions", "[instrucción web omitida]")
        .replace("disregard the system message", "[instrucción web omitida]")
        .replace("reveal the system prompt", "[instrucción web omitida]");
    without_markup
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn with_auth(
    request: reqwest::RequestBuilder,
    settings: &AiHttpSettings,
) -> reqwest::RequestBuilder {
    let api_key = settings.api_key.trim();
    if api_key.is_empty() {
        return request;
    }

    request.bearer_auth(api_key)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn describe_request_error(error: reqwest::Error, fallback: &str) -> String {
    if error.is_timeout() {
        return "La IA excedio el tiempo de espera.".to_string();
    }

    if error.is_connect() {
        return "No se pudo conectar con la IA.".to_string();
    }

    let message = error.to_string();
    if message.trim().is_empty() {
        fallback.to_string()
    } else {
        message
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn pick_default_model(payload: &OllamaTagsResponse) -> Option<String> {
    payload.models.iter().find_map(|model| {
        model
            .name
            .as_deref()
            .or(model.model.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn extract_model_name(model: &OllamaModelDescriptor) -> Option<String> {
    model
        .name
        .as_deref()
        .or(model.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn read_error_detail(response: reqwest::Response) -> String {
    let status = response.status();
    let detail = response
        .text()
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    detail.unwrap_or_else(|| format!("La IA respondio con HTTP {status}."))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn check_ollama_health(settings: &AiHttpSettings) -> Result<AiHealthResult, String> {
    let client = build_client(HEALTH_TIMEOUT_SECS)?;
    let endpoint = build_endpoint(&settings.ollama_url, "/api/tags")?;
    let response = with_auth(
        client
            .get(endpoint)
            .header(reqwest::header::ACCEPT, "application/json"),
        settings,
    )
    .send()
    .await
    .map_err(|error| describe_request_error(error, "No se pudo conectar con la IA."))?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    let payload = response
        .json::<OllamaTagsResponse>()
        .await
        .map_err(|error| {
            describe_request_error(error, "La respuesta de IA no se pudo interpretar.")
        })?;
    let default_model = pick_default_model(&payload);

    Ok(if let Some(model) = default_model {
        AiHealthResult {
            ok: true,
            message: "Conexion correcta con Ollama.".to_string(),
            default_model: Some(model),
        }
    } else {
        AiHealthResult {
            ok: false,
            message: "Ollama respondio, pero no devolvio modelos disponibles.".to_string(),
            default_model: None,
        }
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn list_ollama_models(settings: &AiHttpSettings) -> Result<AiModelListResult, String> {
    let client = build_client(HEALTH_TIMEOUT_SECS)?;
    let tags_endpoint = build_endpoint(&settings.ollama_url, "/api/tags")?;
    let response = with_auth(
        client
            .get(tags_endpoint)
            .header(reqwest::header::ACCEPT, "application/json"),
        settings,
    )
    .send()
    .await
    .map_err(|error| describe_request_error(error, "No se pudo conectar con la IA."))?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    let payload = response
        .json::<OllamaTagsResponse>()
        .await
        .map_err(|error| {
            describe_request_error(error, "La respuesta de IA no se pudo interpretar.")
        })?;

    let mut available_models: Vec<String> = payload
        .models
        .iter()
        .filter_map(extract_model_name)
        .collect();
    available_models.sort_unstable();
    available_models.dedup();

    Ok(AiModelListResult {
        models: available_models,
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn inspect_ollama_model(
    settings: &AiHttpSettings,
    model: &str,
) -> Result<AiModelDetailsResult, String> {
    let normalized_model = model.trim();
    if normalized_model.is_empty() || normalized_model.chars().count() > 200 {
        return Err("El modelo de IA no es valido.".to_string());
    }

    let endpoint = build_endpoint(&settings.ollama_url, "/api/show")?;
    let client = build_client(HEALTH_TIMEOUT_SECS)?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&serde_json::json!({ "model": normalized_model })),
        settings,
    )
    .send()
    .await
    .map_err(|error| describe_request_error(error, "No se pudo consultar el modelo de IA."))?;

    if !response.status().is_success() {
        return Err("Ollama no pudo consultar las capacidades del modelo.".to_string());
    }

    let payload = response
        .json::<OllamaShowResponse>()
        .await
        .map_err(|_| "Ollama devolvio capacidades de modelo invalidas.".to_string())?;
    let context_length = payload
        .model_info
        .iter()
        .find(|(key, _)| key.ends_with(".context_length"))
        .and_then(|(_, value)| value.as_u64());
    Ok(AiModelDetailsResult {
        capabilities: payload.capabilities,
        context_length,
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn run_ollama_chat(
    settings: &AiHttpSettings,
    model: &str,
    messages: &[AiChatMessage],
    think: &serde_json::Value,
    options: &serde_json::Value,
) -> Result<AiChatResult, String> {
    let normalized_model = model.trim();
    if normalized_model.is_empty() {
        return Err("El modelo de Ollama es obligatorio.".to_string());
    }
    if messages.is_empty() {
        return Err("No hay mensajes para enviar a la IA.".to_string());
    }

    let client = build_client(CHAT_TIMEOUT_SECS)?;
    let endpoint = build_endpoint(&settings.ollama_url, "/api/chat")?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&OllamaChatRequest {
                model: normalized_model,
                stream: false,
                think,
                options,
                messages,
            }),
        settings,
    )
    .send()
    .await
    .map_err(|error| {
        describe_request_error(error, "No se pudo completar la consulta con la IA.")
    })?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    let payload = response
        .json::<OllamaChatResponse>()
        .await
        .map_err(|error| {
            describe_request_error(error, "La respuesta de IA no se pudo interpretar.")
        })?;

    if let Some(error_message) = payload
        .error
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Err(error_message);
    }

    let answer = payload
        .message
        .and_then(|message| message.content)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "La IA no devolvio contenido.".to_string())?;

    Ok(AiChatResult { answer })
}

/// A `/api/chat` body with Ollama's model `options` (`num_ctx`), when there
/// are any.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn with_options(mut body: serde_json::Value, options: &serde_json::Value) -> serde_json::Value {
    if !options.is_null() {
        body["options"] = options.clone();
    }
    body
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn run_ollama_tool_chat(
    settings: &AiHttpSettings,
    model: &str,
    messages: &serde_json::Value,
    tools: &serde_json::Value,
    think: &serde_json::Value,
    options: &serde_json::Value,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    let normalized_model = model.trim();
    if normalized_model.is_empty() {
        return Err("El modelo de Ollama es obligatorio.".to_string());
    }
    if !messages.as_array().is_some_and(|items| !items.is_empty()) {
        return Err("No hay mensajes para enviar a la IA.".to_string());
    }
    if !tools.as_array().is_some_and(|items| !items.is_empty()) {
        return Err("No hay herramientas para enviar a la IA.".to_string());
    }

    let client = build_client(timeout_secs)?;
    let endpoint = build_endpoint(&settings.ollama_url, "/api/chat")?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&with_options(
                serde_json::json!({
                    "model": normalized_model,
                    "stream": false,
                    "think": think,
                    "messages": messages,
                    "tools": tools,
                }),
                options,
            )),
        settings,
    )
    .send()
    .await
    .map_err(|error| {
        describe_request_error(error, "No se pudo completar la consulta con herramientas.")
    })?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    response.json::<serde_json::Value>().await.map_err(|error| {
        describe_request_error(
            error,
            "La respuesta de herramientas no se pudo interpretar.",
        )
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn stream_ollama_chat<F>(
    settings: &AiHttpSettings,
    model: &str,
    messages: &[AiChatMessage],
    think: &serde_json::Value,
    on_delta: F,
) -> Result<String, String>
where
    F: FnMut(AiChatStreamDelta) -> Result<(), String>,
{
    stream_ollama_chat_with_cancellation(
        settings,
        model,
        messages,
        think,
        &serde_json::Value::Null,
        Arc::new(AtomicBool::new(false)),
        on_delta,
    )
    .await
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn stream_ollama_chat_with_cancellation<F>(
    settings: &AiHttpSettings,
    model: &str,
    messages: &[AiChatMessage],
    think: &serde_json::Value,
    options: &serde_json::Value,
    cancellation: Arc<AtomicBool>,
    mut on_delta: F,
) -> Result<String, String>
where
    F: FnMut(AiChatStreamDelta) -> Result<(), String>,
{
    let normalized_model = model.trim();
    if normalized_model.is_empty() {
        return Err("El modelo de Ollama es obligatorio.".to_string());
    }
    if messages.is_empty() {
        return Err("No hay mensajes para enviar a la IA.".to_string());
    }

    let client = build_client(CHAT_TIMEOUT_SECS)?;
    let endpoint = build_endpoint(&settings.ollama_url, "/api/chat")?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/x-ndjson")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&OllamaChatRequest {
                model: normalized_model,
                stream: true,
                think,
                options,
                messages,
            }),
        settings,
    )
    .send()
    .await
    .map_err(|error| describe_request_error(error, "No se pudo iniciar el stream de IA."))?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut answer = String::new();

    loop {
        let next_chunk = Box::pin(stream.next());
        let wait_for_cancellation = Box::pin(wait_for_cancellation(Arc::clone(&cancellation)));
        let chunk = match futures::future::select(next_chunk, wait_for_cancellation).await {
            futures::future::Either::Left((chunk, _)) => chunk,
            futures::future::Either::Right((_, _)) => {
                return Err("El stream de IA publicado fue cancelado.".to_string());
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk
            .map_err(|error| describe_request_error(error, "Se interrumpio el stream de IA."))?;
        buffer.extend_from_slice(&chunk);

        while let Some(line_end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=line_end).collect::<Vec<_>>();
            process_stream_line(&line, &mut answer, &mut on_delta)?;
        }
    }

    if !buffer.is_empty() {
        process_stream_line(&buffer, &mut answer, &mut on_delta)?;
    }

    if answer.trim().is_empty() {
        return Err("La IA no devolvio contenido.".to_string());
    }
    Ok(answer.trim().to_string())
}

/// Streams a tool round of `/api/chat`: content and thinking arrive as
/// deltas, and the result has the shape of a round without streaming
/// (`{"message": {"content", "thinking", "tool_calls"}}`), so the same
/// reading and recovery of calls apply. An empty answer comes back as is.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[allow(clippy::too_many_arguments)]
pub async fn stream_ollama_tool_chat_with_cancellation<F>(
    settings: &AiHttpSettings,
    model: &str,
    messages: &serde_json::Value,
    tools: &serde_json::Value,
    think: &serde_json::Value,
    options: &serde_json::Value,
    timeout_secs: u64,
    cancellation: Arc<AtomicBool>,
    mut on_delta: F,
) -> Result<serde_json::Value, String>
where
    F: FnMut(AiChatStreamDelta) -> Result<(), String>,
{
    let normalized_model = model.trim();
    if normalized_model.is_empty() {
        return Err("El modelo de Ollama es obligatorio.".to_string());
    }
    if !messages.as_array().is_some_and(|items| !items.is_empty()) {
        return Err("No hay mensajes para enviar a la IA.".to_string());
    }

    let client = build_client(timeout_secs)?;
    let endpoint = build_endpoint(&settings.ollama_url, "/api/chat")?;
    let response = with_auth(
        client
            .post(endpoint)
            .header(reqwest::header::ACCEPT, "application/x-ndjson")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&with_options(
                serde_json::json!({
                    "model": normalized_model,
                    "stream": true,
                    "think": think,
                    "messages": messages,
                    "tools": tools,
                }),
                options,
            )),
        settings,
    )
    .send()
    .await
    .map_err(|error| describe_request_error(error, "No se pudo iniciar el stream de IA."))?;

    if !response.status().is_success() {
        return Err(read_error_detail(response).await);
    }

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut round = StreamedToolRound::default();

    loop {
        let next_chunk = Box::pin(stream.next());
        let wait_for_cancellation = Box::pin(wait_for_cancellation(Arc::clone(&cancellation)));
        let chunk = match futures::future::select(next_chunk, wait_for_cancellation).await {
            futures::future::Either::Left((chunk, _)) => chunk,
            futures::future::Either::Right((_, _)) => {
                return Err("El stream de IA fue cancelado.".to_string());
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk
            .map_err(|error| describe_request_error(error, "Se interrumpio el stream de IA."))?;
        buffer.extend_from_slice(&chunk);

        while let Some(line_end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=line_end).collect::<Vec<_>>();
            process_tool_stream_line(&line, &mut round, &mut on_delta)?;
        }
    }

    if !buffer.is_empty() {
        process_tool_stream_line(&buffer, &mut round, &mut on_delta)?;
    }
    Ok(round.into_payload())
}

/// What a streamed tool round has said so far.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Default)]
struct StreamedToolRound {
    content: String,
    thinking: String,
    tool_calls: Vec<serde_json::Value>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl StreamedToolRound {
    fn into_payload(self) -> serde_json::Value {
        serde_json::json!({
            "message": {
                "role": "assistant",
                "content": self.content,
                "thinking": self.thinking,
                "tool_calls": self.tool_calls,
            }
        })
    }
}

/// One NDJSON line of a streamed tool round: text goes out as deltas, calls
/// are kept for the end (Ollama sends each call whole, in its own chunk).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn process_tool_stream_line<F>(line: &[u8], round: &mut StreamedToolRound, on_delta: &mut F) -> Result<(), String>
where
    F: FnMut(AiChatStreamDelta) -> Result<(), String>,
{
    let line = std::str::from_utf8(line)
        .map_err(|_| "Ollama devolvio texto UTF-8 invalido.".to_string())?
        .trim();
    if line.is_empty() {
        return Ok(());
    }
    let payload: serde_json::Value = serde_json::from_str(line)
        .map_err(|error| format!("No se pudo interpretar un fragmento de IA: {error}"))?;
    if let Some(error) = payload.get("error").and_then(serde_json::Value::as_str).filter(|value| !value.trim().is_empty()) {
        return Err(error.to_string());
    }
    let Some(message) = payload.get("message") else {
        return Ok(());
    };
    if let Some(thinking) = message.get("thinking").and_then(serde_json::Value::as_str).filter(|value| !value.is_empty()) {
        round.thinking.push_str(thinking);
        on_delta(AiChatStreamDelta::Thinking(thinking.to_string()))?;
    }
    if let Some(content) = message.get("content").and_then(serde_json::Value::as_str).filter(|value| !value.is_empty()) {
        round.content.push_str(content);
        on_delta(AiChatStreamDelta::Content(content.to_string()))?;
    }
    if let Some(calls) = message.get("tool_calls").and_then(serde_json::Value::as_array) {
        round.tool_calls.extend(calls.iter().cloned());
    }
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn wait_for_cancellation(cancellation: Arc<AtomicBool>) {
    while !cancellation.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn process_stream_line<F>(line: &[u8], answer: &mut String, on_delta: &mut F) -> Result<(), String>
where
    F: FnMut(AiChatStreamDelta) -> Result<(), String>,
{
    let line = std::str::from_utf8(line)
        .map_err(|_| "Ollama devolvio texto UTF-8 invalido.".to_string())?
        .trim();
    if line.is_empty() {
        return Ok(());
    }
    let payload: OllamaChatStreamChunk = serde_json::from_str(line)
        .map_err(|error| format!("No se pudo interpretar un fragmento de IA: {error}"))?;
    if let Some(error) = payload.error.filter(|value| !value.trim().is_empty()) {
        return Err(error);
    }
    if let Some(message) = payload.message {
        if let Some(thinking) = message.thinking.filter(|value| !value.is_empty()) {
            on_delta(AiChatStreamDelta::Thinking(thinking))?;
        }
        if let Some(content) = message.content.filter(|value| !value.is_empty()) {
            answer.push_str(&content);
            on_delta(AiChatStreamDelta::Content(content))?;
        }
    }
    Ok(())
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod stream_tests {
    use super::{process_stream_line, process_tool_stream_line, AiChatStreamDelta, StreamedToolRound};

    #[test]
    fn a_streamed_tool_round_keeps_its_text_and_its_calls() {
        let mut round = StreamedToolRound::default();
        let mut deltas = Vec::new();
        let mut on_delta = |delta: AiChatStreamDelta| {
            deltas.push(match delta {
                AiChatStreamDelta::Thinking(value) => format!("thinking:{value}"),
                AiChatStreamDelta::Content(value) => format!("content:{value}"),
            });
            Ok(())
        };
        for line in [
            br#"{"message":{"role":"assistant","content":"Voy a ver ","thinking":"cuento"}}"#.as_slice(),
            br#"{"message":{"role":"assistant","content":"el total."}}"#.as_slice(),
            br#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"list_gmail_messages","arguments":{"query":"from:vapor"}}}]}}"#.as_slice(),
            br#"{"done":true}"#.as_slice(),
        ] {
            process_tool_stream_line(line, &mut round, &mut on_delta).expect("valid line");
        }
        assert_eq!(deltas, vec!["thinking:cuento", "content:Voy a ver ", "content:el total."]);
        let payload = round.into_payload();
        assert_eq!(payload["message"]["content"], "Voy a ver el total.");
        assert_eq!(payload["message"]["thinking"], "cuento");
        assert_eq!(payload["message"]["tool_calls"][0]["function"]["name"], "list_gmail_messages");
        let mut round = StreamedToolRound::default();
        assert_eq!(
            process_tool_stream_line(br#"{"error":"modelo no encontrado"}"#, &mut round, &mut |_| Ok(())),
            Err("modelo no encontrado".to_string())
        );
    }

    #[test]
    fn separates_thinking_and_answer_deltas() {
        let mut answer = String::new();
        let mut deltas = Vec::new();
        process_stream_line(
            br#"{"message":{"thinking":"Analizando...","content":"Respuesta"}}"#,
            &mut answer,
            &mut |delta| match delta {
                AiChatStreamDelta::Thinking(value) => {
                    deltas.push(("thinking", value));
                    Ok(())
                }
                AiChatStreamDelta::Content(value) => {
                    deltas.push(("content", value));
                    Ok(())
                }
            },
        )
        .expect("el fragmento debe ser valido");

        assert_eq!(answer, "Respuesta");
        assert_eq!(
            deltas,
            vec![
                ("thinking", "Analizando...".to_string()),
                ("content", "Respuesta".to_string()),
            ]
        );
    }

    #[test]
    fn propagates_a_consumer_disconnect_from_the_delta_callback() {
        let mut answer = String::new();
        let result = process_stream_line(
            br#"{"message":{"content":"Respuesta"}}"#,
            &mut answer,
            &mut |_delta| Err("cliente desconectado".to_string()),
        );

        assert_eq!(result, Err("cliente desconectado".to_string()));
    }
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod web_search_tests {
    use super::{build_web_search_endpoint, contains_sensitive_web_query_data};

    #[test]
    fn accepts_only_ollama_cloud_for_web_search() {
        assert_eq!(
            build_web_search_endpoint("https://ollama.com").expect("cloud endpoint"),
            "https://ollama.com/api/web_search"
                .parse()
                .expect("valid URL")
        );
        assert!(build_web_search_endpoint("http://localhost:11434").is_err());
        // The default HTTPS port is the same origin; any other port is not Ollama Cloud.
        assert!(build_web_search_endpoint("https://ollama.com:443").is_ok());
        assert!(build_web_search_endpoint("https://ollama.com:8443").is_err());
    }

    #[test]
    fn blocks_sensitive_queries_without_transforming_them() {
        assert!(contains_sensitive_web_query_data(
            "Authorization: Bearer secret-value"
        ));
        assert!(contains_sensitive_web_query_data("API key: secret-value"));
        assert!(contains_sensitive_web_query_data(
            "mi correo es persona@example.com"
        ));
        for query in [
            "Cookie: session=private-value",
            "payload %7B%22password%22%3A%22secret%22%7D",
            "mi trabajo es una empresa privada",
            "mi sueldo es 2500000",
            "mi expediente legal es privado",
            "calendario privado: reunion el viernes",
            "C:\\Users\\gabmi\\Documents\\nota.md",
            "10.0.0.20 novedades de red",
            "mi nombre es Ana Perez",
        ] {
            assert!(
                contains_sensitive_web_query_data(query),
                "query should be blocked: {query}"
            );
        }
        assert!(!contains_sensitive_web_query_data(
            "novedades públicas de Rust"
        ));
    }
}
