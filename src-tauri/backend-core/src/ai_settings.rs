//! AI provider preferences and model capabilities.
//!
//! The settings screen sends what the person typed; this module normalizes
//! it (endpoint, model, thinking), describes what each model can do from the
//! provider's capabilities (with name-based hints where the provider does
//! not report them) and holds the prompts of the one-shot AI tasks.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::BackendError;

pub const DEFAULT_OLLAMA_URL: &str = "https://ollama.com";

/// Endpoints of older versions that meant "use the default provider".
const LEGACY_DEFAULT_URLS: [&str; 5] = [
    "http://127.0.0.1:8000",
    "http://127.0.0.1:9991",
    "http://127.0.0.1:9991/api",
    "http://localhost:9991",
    "http://localhost:9991/api",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Low,
    #[default]
    Medium,
    High,
}

impl ThinkingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

fn default_true() -> bool {
    true
}

/// Provider preferences as the settings screen edits them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsInput {
    #[serde(default)]
    pub ollama_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub selected_model: String,
    /// Model that reads the images; empty means the selected one.
    #[serde(default)]
    pub image_model: String,
    #[serde(default = "default_true")]
    pub thinking_enabled: bool,
    #[serde(default, deserialize_with = "lenient_thinking_level")]
    pub thinking_level: ThinkingLevel,
}

fn lenient_thinking_level<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<ThinkingLevel, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(match value.as_str() {
        Some("low") => ThinkingLevel::Low,
        Some("high") => ThinkingLevel::High,
        _ => ThinkingLevel::Medium,
    })
}

/// Normalized provider preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSettings {
    pub ollama_url: String,
    pub api_key: String,
    pub selected_model: String,
    pub image_model: String,
    pub thinking_enabled: bool,
    pub thinking_level: ThinkingLevel,
}

impl AiSettings {
    /// The model that reads the images when it is not the selected one.
    pub fn separate_image_model(&self) -> Option<&str> {
        separate_image_model(&self.selected_model, &self.image_model)
    }
}

/// `image_model` when it is set and differs from `selected_model`: images
/// then go to it and the selected model gets what it read from them.
pub fn separate_image_model<'a>(selected_model: &str, image_model: &'a str) -> Option<&'a str> {
    let image_model = image_model.trim();
    (!image_model.is_empty() && image_model != selected_model.trim()).then_some(image_model)
}

/// Instructions of the image model: what it reads from an image reaches a
/// model that cannot see it, so it describes everything and solves nothing.
pub const IMAGE_READER_PROMPT: &str = "Sos el lector de imágenes de un asistente. Otro modelo, que no puede ver la imagen, va a resolver el pedido de la persona solo con lo que escribas. Describí la imagen de forma completa y fiel:
- Transcribí textualmente todo el texto y los números que se lean (tickets, facturas, resúmenes, recibos, documentos, pantallas, carteles), respetando el orden, las líneas, las columnas y los totales. Si algo no se lee bien, decilo en lugar de adivinarlo.
- Si es comida: qué plato es, los ingredientes que se ven, el tamaño aproximado de la porción y cómo parece estar cocinado.
- Si es otra cosa: qué muestra (personas, objetos, lugar, gráfico, captura de pantalla, apuntes) y los detalles que alguien necesitaría para actuar sobre ella.
No respondas ni resuelvas ningún pedido: solo describí. Escribí en español.";

impl AiSettingsInput {
    pub fn normalize(&self) -> AiSettings {
        AiSettings {
            ollama_url: normalize_ollama_url(&self.ollama_url),
            api_key: self.api_key.trim().to_string(),
            selected_model: self.selected_model.trim().to_string(),
            image_model: self.image_model.trim().to_string(),
            thinking_enabled: self.thinking_enabled,
            thinking_level: self.thinking_level,
        }
    }
}

/// `scheme://host[:port]` of an http(s) endpoint without credentials, or the
/// default provider for anything else.
pub fn normalize_ollama_url(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() || LEGACY_DEFAULT_URLS.contains(&raw) {
        return DEFAULT_OLLAMA_URL.to_string();
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return DEFAULT_OLLAMA_URL.to_string();
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return DEFAULT_OLLAMA_URL.to_string();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return DEFAULT_OLLAMA_URL.to_string();
    }
    let normalized = format!("{scheme}://{}", authority.to_ascii_lowercase());
    if LEGACY_DEFAULT_URLS.contains(&normalized.as_str()) {
        DEFAULT_OLLAMA_URL.to_string()
    } else {
        normalized
    }
}

fn contains_any(model: &str, tokens: &[&str]) -> bool {
    let model = model.trim().to_lowercase();
    !model.is_empty() && tokens.iter().any(|token| model.contains(token))
}

fn likely_vision(model: &str) -> bool {
    contains_any(
        model,
        &["vision", "vl", "llava", "bakllava", "moondream", "minicpm-v", "gemma3", "gemma4", "gemini", "glm-ocr", "qwen3.5"],
    )
}

fn likely_thinking(model: &str) -> bool {
    contains_any(model, &["deepseek-r1", "gpt-oss", "qwen3", "qwq", "reasoning"])
}

fn likely_tools(model: &str) -> bool {
    contains_any(
        model,
        &["qwen3.5", "qwen3.6", "gemma4", "granite4", "devstral", "hermes3", "llama3-groq-tool-use", "lfm2", "nemotron3"],
    )
}

/// Models that take a thinking level instead of on/off.
pub fn supports_thinking_levels(model: &str) -> bool {
    contains_any(model, &["gpt-oss"])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelOptionDto {
    pub name: String,
    pub supports_thinking: bool,
    pub supports_thinking_levels: bool,
    pub supports_vision: bool,
    pub supports_tools: bool,
}

/// How much to trust the capabilities a provider reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilitySource {
    /// The provider's list is complete (Android bridge): nothing is guessed.
    Exact,
    /// The provider may omit capabilities (desktop Ollama and Ollama Cloud):
    /// name hints complete them.
    Partial,
}

/// Model option from the provider's capabilities; `None` when they could not
/// be read.
pub fn model_option(name: &str, capabilities: Option<&[String]>, source: CapabilitySource) -> AiModelOptionDto {
    let has = |capability: &str| capabilities.is_some_and(|list| list.iter().any(|item| item == capability));
    match source {
        CapabilitySource::Exact => AiModelOptionDto {
            name: name.to_string(),
            supports_thinking: has("thinking"),
            supports_thinking_levels: false,
            supports_vision: has("vision"),
            supports_tools: has("tools"),
        },
        CapabilitySource::Partial => AiModelOptionDto {
            name: name.to_string(),
            supports_thinking: has("thinking") || likely_thinking(name),
            supports_thinking_levels: supports_thinking_levels(name),
            supports_vision: has("vision") || likely_vision(name),
            supports_tools: has("tools") || likely_tools(name),
        },
    }
}

/// Value of Ollama's `think` field for a model.
pub fn think_value(settings: &AiSettings, model: &str) -> Value {
    if !settings.thinking_enabled {
        Value::Bool(false)
    } else if supports_thinking_levels(model) {
        Value::String(settings.thinking_level.as_str().to_string())
    } else {
        Value::Bool(true)
    }
}

/// Model a one-shot task uses: the selected one, or the first available.
pub fn resolve_model(settings: &AiSettings, available: &[String]) -> Result<String, BackendError> {
    if !settings.selected_model.is_empty() {
        return Ok(settings.selected_model.clone());
    }
    available
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .min()
        .map(str::to_string)
        .ok_or_else(|| BackendError::invalid_input("No hay modelos disponibles en Ollama."))
}

/// Context tokens reserved for the task's answer, over the prompt. The
/// answer itself is not capped: a model that writes its reasoning before
/// the answer needs more, and the task's timeout stops a runaway one.
pub const TASK_OUTPUT_TOKENS: usize = 4_096;
/// Context sizes a task asks for, smallest first. Few sizes, so a local
/// model is not reloaded on every request for a slightly different one.
/// A request larger than every step asks for what it needs: the only
/// ceiling is the model's own context.
const TASK_CONTEXT_TOKENS: [usize; 4] = [16_384, 32_768, 65_536, 131_072];
/// Characters per token, on the safe side for Spanish text.
const CHARS_PER_TOKEN: usize = 3;
/// Tokens an attached image takes.
const IMAGE_TOKENS: usize = 1_536;

/// Ollama `options` of a one-shot task. Without `num_ctx` a local model
/// reads only its default context (2048 tokens) and drops the start of a
/// longer prompt, with its instructions: it answers something else, or
/// writes until the request times out. `max_context` is the model's own
/// context limit, when Ollama reports it: the request never asks beyond
/// it, and nothing else caps the size. Cloud models ignore both values.
pub fn task_options(prompt_chars: usize, images: usize, max_context: Option<usize>) -> Value {
    let needed = prompt_chars.div_ceil(CHARS_PER_TOKEN) + images * IMAGE_TOKENS + TASK_OUTPUT_TOKENS;
    let num_ctx = TASK_CONTEXT_TOKENS.into_iter().find(|size| *size >= needed).unwrap_or(needed);
    let num_ctx = max_context.map_or(num_ctx, |max| num_ctx.min(max));
    serde_json::json!({ "num_ctx": num_ctx })
}

pub const INKMATH_SYSTEM_PROMPT: &str = "Sos un transcriptor preciso de formulas matematicas manuscritas a LaTeX.";
pub const INKMATH_USER_PROMPT: &str = "Transcribi exclusivamente la formula matematica manuscrita de la imagen a LaTeX. Responde solo con el codigo LaTeX, sin delimitadores, bloques Markdown ni explicaciones. Conserva fracciones, indices, exponentes, raices, integrales, sumatorias y saltos de linea visibles. No inventes simbolos que no aparezcan en la imagen.";

/// LaTeX of an InkMath answer without code fences or `$$` delimiters.
pub fn clean_latex_answer(answer: &str) -> Option<String> {
    let mut latex = answer.trim();
    for prefix in ["```latex", "```tex", "```"] {
        if let Some(rest) = latex.strip_prefix(prefix) {
            latex = rest.trim_start();
            break;
        }
    }
    latex = latex.strip_suffix("```").unwrap_or(latex).trim_end();
    if let Some(inner) = latex.strip_prefix("$$").and_then(|rest| rest.strip_suffix("$$")) {
        latex = inner;
    }
    let latex = latex.trim();
    (!latex.is_empty()).then(|| latex.to_string())
}

pub const TRANSCRIPT_SYSTEM_PROMPT: &str = "Sos el asistente de Notia. Responde con claridad, prioriza el contexto provisto y usa markdown solo cuando aporte valor.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_asks_for_a_context_that_holds_its_prompt_and_answer() {
        let options = |chars: usize, images: usize, max: Option<usize>| task_options(chars, images, max)["num_ctx"].as_u64().unwrap();
        // A meeting's notes request: transcript, library and earlier notes.
        assert_eq!(options(34_000, 0, None), 16_384);
        assert_eq!(options(40_000, 0, Some(262_144)), 32_768);
        assert_eq!(options(1_000, 2, None), 16_384);
        // Larger than every step: what it needs, up to the model's context.
        assert_eq!(options(1_000_000, 0, None), 337_430);
        assert_eq!(options(1_000_000, 0, Some(262_144)), 262_144);
        // A model smaller than the step the request would take.
        assert_eq!(options(40_000, 0, Some(8_192)), 8_192);
        // The answer is never capped: a model may reason before it answers.
        assert!(task_options(10, 0, None).get("num_predict").is_none());
    }

    #[test]
    fn endpoints_are_reduced_to_scheme_and_host() {
        assert_eq!(normalize_ollama_url(""), DEFAULT_OLLAMA_URL);
        assert_eq!(normalize_ollama_url("http://localhost:9991/api"), DEFAULT_OLLAMA_URL);
        assert_eq!(normalize_ollama_url("ftp://host"), DEFAULT_OLLAMA_URL);
        assert_eq!(normalize_ollama_url("http://user:pw@host:11434"), DEFAULT_OLLAMA_URL);
        assert_eq!(normalize_ollama_url(" HTTP://Host:11434/api/tags?x=1 "), "http://host:11434");
        assert_eq!(normalize_ollama_url("https://ollama.com/"), "https://ollama.com");
    }

    #[test]
    fn capabilities_are_exact_on_android_and_completed_by_hints_elsewhere() {
        let reported = vec!["tools".to_string()];
        let exact = model_option("qwen3.5:9b", Some(&reported), CapabilitySource::Exact);
        assert!(exact.supports_tools && !exact.supports_vision && !exact.supports_thinking);
        let partial = model_option("qwen3.5:9b", Some(&reported), CapabilitySource::Partial);
        assert!(partial.supports_tools && partial.supports_vision && partial.supports_thinking);
        let unknown = model_option("gpt-oss:20b", None, CapabilitySource::Partial);
        assert!(unknown.supports_thinking_levels);
    }

    #[test]
    fn think_follows_the_preference_and_the_model() {
        let mut settings = AiSettingsInput::default().normalize();
        settings.thinking_enabled = true;
        settings.thinking_level = ThinkingLevel::High;
        assert_eq!(think_value(&settings, "gpt-oss:20b"), Value::String("high".into()));
        assert_eq!(think_value(&settings, "qwen3:8b"), Value::Bool(true));
        settings.thinking_enabled = false;
        assert_eq!(think_value(&settings, "gpt-oss:20b"), Value::Bool(false));
    }

    #[test]
    fn latex_answers_lose_fences_and_delimiters() {
        assert_eq!(clean_latex_answer("```latex\n\\frac{a}{b}\n```").as_deref(), Some("\\frac{a}{b}"));
        assert_eq!(clean_latex_answer("$$x^2$$").as_deref(), Some("x^2"));
        assert_eq!(clean_latex_answer("  "), None);
    }

    #[test]
    fn the_image_model_counts_only_when_it_is_another_one() {
        let mut settings = AiSettingsInput { selected_model: " qwen3 ".into(), image_model: " gemma3 ".into(), ..Default::default() }.normalize();
        assert_eq!(settings.image_model, "gemma3");
        assert_eq!(settings.separate_image_model(), Some("gemma3"));
        settings.image_model = "qwen3".into();
        assert_eq!(settings.separate_image_model(), None);
        settings.image_model.clear();
        assert_eq!(settings.separate_image_model(), None);
        let input: AiSettingsInput = serde_json::from_value(serde_json::json!({ "selectedModel": "a" })).expect("older settings");
        assert_eq!(input.image_model, "");
    }

    #[test]
    fn a_selected_model_wins_over_the_available_list() {
        let mut settings = AiSettingsInput::default().normalize();
        assert_eq!(resolve_model(&settings, &["b".into(), "a".into()]).unwrap(), "a");
        settings.selected_model = "c".into();
        assert_eq!(resolve_model(&settings, &[]).unwrap(), "c");
        settings.selected_model.clear();
        assert!(resolve_model(&settings, &[]).is_err());
    }
}
