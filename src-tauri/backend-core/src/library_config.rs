//! Library configuration (`.notia/notiaConfig.json`): normalization and
//! defaults. The backend owns the persisted shape; clients only send the
//! sections they edit and render what comes back.

use serde_json::{json, Map, Value};

use crate::error::BackendError;

/// Logical path of the configuration inside a library.
pub const LIBRARY_CONFIG_LOGICAL_PATH: &str = ".notia/notiaConfig.json";
/// Directory that holds the configuration and other Notia metadata.
pub const LIBRARY_CONFIG_DIRECTORY: &str = ".notia";
/// Upper bound of the serialized configuration.
pub const MAX_LIBRARY_CONFIG_BYTES: usize = 1024 * 1024;

const CONTEXT_DEFAULTS_VERSION: u64 = 1;
const DEFAULT_REFRESH_INTERVAL_MS: u64 = 30_000;
const MAX_REFRESH_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;
const DEFAULT_CONTEXT_COLOR: &str = "#64748B";
const MAX_LLAMACLOUD_API_KEY_CHARS: usize = 256;
const MAX_TELEGRAM_TOKEN_CHARS: usize = 256;
const MAX_CONTEXTS: usize = 200;

/// Tag assigned to new notes and to documents without a context.
pub const DEFAULT_CONTEXT_TAG: &str = "#Personal";

/// Default context catalog of every library.
pub const DEFAULT_LIBRARY_CONTEXTS: [(&str, &str); 5] = [
    ("#Laboral", "#2563EB"),
    ("#Personal", "#16A34A"),
    ("#Academico", "#9333EA"),
    ("#Confidencial", "#DC2626"),
    ("#Publico", "#D97706"),
];

/// Contexts every library always has: they cannot be removed or renamed
/// away. A configuration that lacks one gets it back with its color.
pub const PROTECTED_CONTEXT_TAGS: [&str; 2] = ["#Confidencial", "#Publico"];

/// Context of the files only the AI writes (`.agent/memory`). Notia sets
/// it; it is not part of the catalog, so it cannot be chosen for a note or
/// a folder, assigned to a user nor created in Settings. The Owner, who
/// has every context, reads those files; nobody edits them by hand.
pub const SYSTEM_CONTEXT_TAG: &str = "#System";

/// Whether `tag` is the reserved `#System` context.
pub fn is_system_context(tag: &str) -> bool {
    let tag = tag.trim();
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    tag.eq_ignore_ascii_case(&SYSTEM_CONTEXT_TAG[1..])
}

/// Whether `tag` is one of the contexts that cannot be removed.
pub fn is_protected_context(tag: &str) -> bool {
    PROTECTED_CONTEXT_TAGS.iter().any(|protected| protected.eq_ignore_ascii_case(tag.trim()))
}

/// Result of normalizing a stored configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedLibraryConfig {
    pub config: Value,
    /// The stored file predates the current defaults and must be rewritten.
    pub needs_migration: bool,
}

/// Configuration written for a library that has none.
pub fn default_library_config() -> Value {
    json!({
        "version": 1,
        "contextDefaultsVersion": CONTEXT_DEFAULTS_VERSION,
        "panelDesplegable": { "refreshIntervalMs": DEFAULT_REFRESH_INTERVAL_MS },
        "contexts": default_contexts(),
    })
}

/// Parses and normalizes the stored configuration text.
pub fn parse_library_config(text: &str) -> Result<NormalizedLibraryConfig, BackendError> {
    if text.len() > MAX_LIBRARY_CONFIG_BYTES {
        return Err(BackendError::invalid_input(
            "La configuración de la biblioteca supera el tamaño permitido.",
        ));
    }
    let value = serde_json::from_str::<Value>(text).map_err(|_| {
        BackendError::invalid_input("La configuración de la biblioteca no es JSON válido.")
    })?;
    Ok(normalize_library_config(&value))
}

/// Normalizes any value into the persisted configuration shape. Unknown or
/// legacy sections (for example old publication credentials) are dropped.
pub fn normalize_library_config(value: &Value) -> NormalizedLibraryConfig {
    let Some(candidate) = value.as_object() else {
        return NormalizedLibraryConfig {
            config: default_library_config(),
            needs_migration: true,
        };
    };
    let current_defaults =
        candidate.get("contextDefaultsVersion").and_then(Value::as_u64) == Some(CONTEXT_DEFAULTS_VERSION);

    let mut config = Map::new();
    config.insert(
        "version".into(),
        candidate
            .get("version")
            .filter(|version| version.is_number())
            .cloned()
            .unwrap_or_else(|| json!(1)),
    );
    config.insert("contextDefaultsVersion".into(), json!(CONTEXT_DEFAULTS_VERSION));
    config.insert(
        "panelDesplegable".into(),
        json!({ "refreshIntervalMs": normalize_refresh_interval(candidate.get("panelDesplegable")) }),
    );
    // InkMath preferences are presentation settings; only their container
    // shape is enforced.
    if let Some(object) = candidate.get("inkMath").filter(|value| value.is_object()) {
        config.insert("inkMath".into(), object.clone());
    }
    if let Some(ai) = candidate.get("ia").filter(|value| value.is_object()) {
        config.insert("ia".into(), normalize_ai(ai));
    }
    if let Some(telegram) = candidate.get("telegram").filter(|value| !value.is_null()) {
        config.insert("telegram".into(), normalize_telegram(telegram));
    }
    let contexts = if current_defaults {
        ensure_protected_contexts(normalize_contexts(candidate.get("contexts")))
    } else {
        ensure_default_contexts(candidate.get("contexts"))
    };
    config.insert("contexts".into(), Value::Array(contexts));
    if let Some(llamacloud) = normalize_llamacloud(candidate.get("llamacloud")) {
        config.insert("llamacloud".into(), llamacloud);
    }
    // Google Cloud client and connected mail accounts; clients never
    // receive them.
    if let Some(client) = crate::mail_accounts::normalize_google_cloud(candidate.get(crate::mail_accounts::GOOGLE_CLOUD_KEY)) {
        config.insert(crate::mail_accounts::GOOGLE_CLOUD_KEY.into(), client);
    }
    if let Some(accounts) = crate::mail_accounts::normalize_mail_accounts(candidate.get(crate::mail_accounts::MAIL_ACCOUNTS_KEY)) {
        config.insert(crate::mail_accounts::MAIL_ACCOUNTS_KEY.into(), accounts);
    }
    // The WhatsApp link and what each chat allows; clients never receive it.
    if let Some(whatsapp) = crate::whatsapp::normalize_whatsapp_config(candidate.get(crate::whatsapp::WHATSAPP_KEY)) {
        config.insert(crate::whatsapp::WHATSAPP_KEY.into(), whatsapp);
    }
    // The webhook endpoint and its actions; clients never receive it.
    if let Some(webhook) = crate::webhooks::normalize_webhook_config(candidate.get(crate::webhooks::WEBHOOK_KEY)) {
        config.insert(crate::webhooks::WEBHOOK_KEY.into(), webhook);
    }
    // The weather place, chosen in Settings among Open-Meteo's places.
    if let Some(weather) = crate::weather::normalize_weather_config(candidate.get(crate::weather::WEATHER_KEY)) {
        config.insert(crate::weather::WEATHER_KEY.into(), weather);
    }

    NormalizedLibraryConfig {
        config: Value::Object(config),
        needs_migration: !current_defaults,
    }
}

/// Serializes a normalized configuration for persistence.
pub fn serialize_library_config(config: &Value) -> Result<String, BackendError> {
    let text = serde_json::to_string_pretty(config).map_err(|_| {
        BackendError::invalid_input("No se pudo serializar la configuración de la biblioteca.")
    })?;
    if text.len() > MAX_LIBRARY_CONFIG_BYTES {
        return Err(BackendError::invalid_input(
            "La configuración de la biblioteca supera el tamaño permitido.",
        ));
    }
    Ok(text)
}

fn normalize_refresh_interval(panel: Option<&Value>) -> u64 {
    panel
        .and_then(|panel| panel.get("refreshIntervalMs"))
        .and_then(Value::as_u64)
        .filter(|interval| *interval > 0 && *interval <= MAX_REFRESH_INTERVAL_MS)
        .unwrap_or(DEFAULT_REFRESH_INTERVAL_MS)
}

fn default_contexts() -> Vec<Value> {
    DEFAULT_LIBRARY_CONTEXTS
        .iter()
        .map(|(tag, color)| json!({ "tag": tag, "color": color }))
        .collect()
}

/// `#tag` without whitespace or additional `#`.
pub fn normalize_context_tag(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tag = if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("#{trimmed}")
    };
    let body = &tag[1..];
    (!body.is_empty() && !body.contains('#') && !body.chars().any(char::is_whitespace)).then_some(tag)
}

fn normalize_context_color(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|color| {
            color.len() == 7
                && color.starts_with('#')
                && color[1..].chars().all(|character| character.is_ascii_hexdigit())
        })
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| DEFAULT_CONTEXT_COLOR.to_string())
}

fn normalize_contexts(value: Option<&Value>) -> Vec<Value> {
    let Some(items) = value.and_then(Value::as_array) else {
        return default_contexts();
    };
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .filter_map(|item| {
            let tag = normalize_context_tag(item.get("tag")?.as_str()?).filter(|tag| !is_system_context(tag))?;
            seen.insert(tag.to_lowercase()).then(|| {
                json!({ "tag": tag, "color": normalize_context_color(item.get("color")) })
            })
        })
        .take(MAX_CONTEXTS)
        .collect()
}

/// Adds back the protected contexts a configuration lost (removed or renamed).
fn ensure_protected_contexts(mut contexts: Vec<Value>) -> Vec<Value> {
    for (tag, color) in DEFAULT_LIBRARY_CONTEXTS.iter().filter(|(tag, _)| is_protected_context(tag)) {
        let present = contexts
            .iter()
            .any(|context| context.get("tag").and_then(Value::as_str).is_some_and(|known| known.eq_ignore_ascii_case(tag)));
        if !present {
            contexts.push(json!({ "tag": tag, "color": color }));
        }
    }
    contexts
}

fn ensure_default_contexts(value: Option<&Value>) -> Vec<Value> {
    let mut contexts = normalize_contexts(value);
    let known = contexts
        .iter()
        .filter_map(|context| context.get("tag").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect::<std::collections::HashSet<_>>();
    for (tag, color) in DEFAULT_LIBRARY_CONTEXTS {
        if !known.contains(&tag.to_lowercase()) {
            contexts.push(json!({ "tag": tag, "color": color }));
        }
    }
    contexts
}

fn normalize_llamacloud(value: Option<&Value>) -> Option<Value> {
    let api_key = value?.get("apiKey")?.as_str()?.trim();
    (!api_key.is_empty() && api_key.chars().count() <= MAX_LLAMACLOUD_API_KEY_CHARS)
        .then(|| json!({ "apiKey": api_key }))
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Provider preferences of the library: normalized provider settings (older
/// files used `baseUrl` and `model`) and how Telegram shows progress.
fn normalize_ai(value: &Value) -> Value {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    let settings = crate::ai_settings::AiSettingsInput {
        ollama_url: text(&["ollamaUrl", "baseUrl"]),
        api_key: text(&["apiKey"]),
        selected_model: text(&["selectedModel", "model"]),
        image_model: text(&["imageModel"]),
        thinking_enabled: value.get("thinkingEnabled").and_then(Value::as_bool) != Some(false),
        thinking_level: match value.get("thinkingLevel").and_then(Value::as_str) {
            Some("low") => crate::ai_settings::ThinkingLevel::Low,
            Some("high") => crate::ai_settings::ThinkingLevel::High,
            _ => crate::ai_settings::ThinkingLevel::Medium,
        },
    }
    .normalize();
    let progress_mode = match value.get("progressMode").and_then(Value::as_str) {
        Some(mode @ ("minimal" | "standard" | "detailed" | "off")) => mode,
        _ => "minimal",
    };
    let enabled = |key: &str| value.get(key).and_then(Value::as_bool) != Some(false);
    json!({
        "ollamaUrl": settings.ollama_url,
        "apiKey": settings.api_key,
        "selectedModel": settings.selected_model,
        "imageModel": settings.image_model,
        "thinkingEnabled": settings.thinking_enabled,
        "thinkingLevel": settings.thinking_level.as_str(),
        "progressMode": progress_mode,
        "showPlan": enabled("showPlan"),
        "showReasoningSummary": enabled("showReasoningSummary"),
        "editProgressMessage": enabled("editProgressMessage"),
    })
}

/// Telegram bot of the library. The backend worker keeps the polling offset
/// and the processed updates in its own state, not in the configuration.
/// `autonomousAgent` (on unless turned off) lets the agent write to the
/// Owner by itself when new mail arrives. The hourly review became an AI
/// action; the switch only decides whether that action starts on.
fn normalize_telegram(value: &Value) -> Value {
    json!({
        "enabled": value.get("enabled").and_then(Value::as_bool) == Some(true),
        "botToken": truncate_chars(
            value.get("botToken").and_then(Value::as_str).unwrap_or_default().trim(),
            MAX_TELEGRAM_TOKEN_CHARS,
        ),
        "autonomousAgent": value.get("autonomousAgent").and_then(Value::as_bool) != Some(false),
    })
}

/// Whether the library's agent may write to the Owner by itself.
pub fn autonomous_agent_enabled(config: &Value) -> bool {
    config.pointer("/telegram/autonomousAgent").and_then(Value::as_bool) != Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_accounts_survive_normalization_and_invalid_ones_are_dropped() {
        let account = json!({ "email": "ana@gmail.com", "accessToken": "a", "refreshToken": "r", "expiresAtMs": 1, "connectedAtMs": 1 });
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "googleCloud": { "clientId": "1-a.apps.googleusercontent.com", "clientSecret": "s" },
            "mailAccounts": { "gmail": account, "outlook": { "email": "sin token" } },
        }));
        assert_eq!(normalized.config["googleCloud"]["clientSecret"], "s");
        assert_eq!(normalized.config["mailAccounts"][0]["refreshToken"], "r");
        assert_eq!(normalized.config["mailAccounts"].as_array().map(Vec::len), Some(1));
        assert!(normalize_library_config(&json!({ "mailAccounts": {} })).config.get("mailAccounts").is_none());
    }

    #[test]
    fn non_objects_become_the_default_configuration() {
        let normalized = normalize_library_config(&json!("x"));
        assert!(normalized.needs_migration);
        assert_eq!(normalized.config, default_library_config());
    }

    #[test]
    fn legacy_configs_gain_default_contexts_and_are_migrated() {
        let normalized = normalize_library_config(&json!({
            "version": 1,
            "contexts": [{ "tag": "Viajes", "color": "#abcdef" }],
        }));
        assert!(normalized.needs_migration);
        let contexts = normalized.config["contexts"].as_array().expect("contexts");
        assert_eq!(contexts[0], json!({ "tag": "#Viajes", "color": "#ABCDEF" }));
        assert_eq!(contexts.len(), 1 + DEFAULT_LIBRARY_CONTEXTS.len());
    }

    #[test]
    fn current_configs_keep_removed_default_contexts_removed_but_the_protected_ones() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "contexts": [{ "tag": "#Uno" }, { "tag": "#uno" }, { "tag": "dos tres" }],
        }));
        assert!(!normalized.needs_migration);
        assert_eq!(
            normalized.config["contexts"],
            json!([
                { "tag": "#Uno", "color": DEFAULT_CONTEXT_COLOR },
                { "tag": "#Confidencial", "color": "#DC2626" },
                { "tag": "#Publico", "color": "#D97706" },
            ])
        );
    }

    #[test]
    fn protected_contexts_survive_removal_and_renaming_and_keep_their_color() {
        // «Confidencial» renamed and «Publico» removed: both come back; the
        // new name stays as another context.
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "contexts": [{ "tag": "#Secreto", "color": "#DC2626" }, { "tag": "#confidencial", "color": "#111111" }],
        }));
        assert_eq!(
            normalized.config["contexts"],
            json!([
                { "tag": "#Secreto", "color": "#DC2626" },
                { "tag": "#confidencial", "color": "#111111" },
                { "tag": "#Publico", "color": "#D97706" },
            ])
        );
        assert!(is_protected_context(" #PUBLICO "));
        assert!(!is_protected_context("#Personal"));
    }

    #[test]
    fn the_system_context_is_reserved_and_never_in_the_catalog() {
        assert!(is_system_context("#System") && is_system_context(" system ") && is_system_context("#SYSTEM"));
        assert!(!is_system_context("#Systems") && !is_system_context("#Confidencial"));
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "contexts": [{ "tag": "#System", "color": "#64748B" }, { "tag": "#Personal", "color": "#16A34A" }],
        }));
        let tags = normalized.config["contexts"].as_array().expect("contexts").iter().filter_map(|context| context["tag"].as_str()).collect::<Vec<_>>();
        assert!(tags.contains(&"#Personal") && !tags.iter().any(|tag| is_system_context(tag)), "{tags:?}");
    }

    #[test]
    fn drops_unknown_sections_and_invalid_credentials() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "taskManagerPublication": { "accessUsers": [{ "passwordHash": "x" }] },
            "llamacloud": { "apiKey": "   " },
            "ia": { "apiKey": "clave", "selectedModel": "qwen3" },
        }));
        let config = normalized.config.as_object().expect("object");
        assert!(!config.contains_key("taskManagerPublication"));
        assert!(!config.contains_key("llamacloud"));
        assert_eq!(config["ia"]["apiKey"], "clave");
    }

    #[test]
    fn telegram_keeps_only_the_switches_and_a_bounded_token() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "telegram": {
                "enabled": true,
                "botToken": format!("  {}  ", "t".repeat(300)),
                "updateOffset": 42,
                "processedUpdateIds": [1, 2],
                "authorizedPeer": { "chatId": 1, "userId": 2, "displayName": "Ana" },
            },
        }));
        let telegram = &normalized.config["telegram"];
        assert_eq!(telegram["enabled"], true);
        assert_eq!(telegram["botToken"].as_str().expect("token").len(), MAX_TELEGRAM_TOKEN_CHARS);
        assert_eq!(telegram["autonomousAgent"], true);
        assert_eq!(telegram.as_object().expect("object").len(), 3);
        assert!(autonomous_agent_enabled(&normalized.config));
        assert!(autonomous_agent_enabled(&json!({})));
    }

    #[test]
    fn the_autonomous_agent_stays_off_once_turned_off() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "telegram": { "enabled": true, "botToken": "1:a", "autonomousAgent": false },
        }));
        assert_eq!(normalized.config["telegram"]["autonomousAgent"], false);
        assert!(!autonomous_agent_enabled(&normalized.config));
    }

    #[test]
    fn ai_preferences_are_normalized_and_legacy_keys_are_read() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "ia": { "baseUrl": " http://Host:11434/api ", "model": " qwen3 ", "thinkingLevel": "x", "showPlan": false },
        }));
        let ai = &normalized.config["ia"];
        assert_eq!(ai["ollamaUrl"], "http://host:11434");
        assert_eq!(ai["selectedModel"], "qwen3");
        assert_eq!(ai["thinkingLevel"], "medium");
        assert_eq!(ai["progressMode"], "minimal");
        assert_eq!(ai["showPlan"], false);
        assert_eq!(ai["thinkingEnabled"], true);
        assert_eq!(ai["imageModel"], "");
        let with_reader = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "ia": { "selectedModel": "qwen3", "imageModel": " gemma3 " },
        }));
        assert_eq!(with_reader.config["ia"]["imageModel"], "gemma3");
    }

    #[test]
    fn refresh_interval_falls_back_when_invalid() {
        let normalized = normalize_library_config(&json!({
            "contextDefaultsVersion": 1,
            "panelDesplegable": { "refreshIntervalMs": 0 },
        }));
        assert_eq!(
            normalized.config["panelDesplegable"]["refreshIntervalMs"],
            DEFAULT_REFRESH_INTERVAL_MS
        );
    }

    #[test]
    fn rejects_oversized_or_invalid_text() {
        assert!(parse_library_config("{").is_err());
        assert!(parse_library_config(&" ".repeat(MAX_LIBRARY_CONFIG_BYTES + 1)).is_err());
    }
}
