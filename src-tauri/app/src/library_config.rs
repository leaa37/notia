//! Tauri commands for the library configuration. The backend reads,
//! normalizes, migrates and persists `.notia/notiaConfig.json` through the
//! registered library binding; the WebView only sends library identity and
//! the configuration it wants to store.
//!
//! Once the Owner has a password the file is encrypted with it (see
//! `backend_core::config_envelope`): it is read and written only while the
//! library is unlocked (`config_vault`). A file from before the password is
//! plain text and is sealed the first time the Owner signs in.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::host::{AppHandle, Manager};

use crate::backend::config_envelope::{classify_stored_config, locked_error, serialize_envelope, ConfigEnvelope, StoredConfig};
use crate::backend::library_config::{
    default_library_config, normalize_library_config, parse_library_config,
    serialize_library_config, LIBRARY_CONFIG_DIRECTORY, LIBRARY_CONFIG_LOGICAL_PATH,
};
use crate::backend::mail_accounts::{without_google_secrets, GOOGLE_CLOUD_KEY, MAIL_ACCOUNTS_KEY};
use crate::backend::webhooks::{without_webhook, WEBHOOK_KEY};
use crate::backend::whatsapp::{without_whatsapp, WHATSAPP_KEY};
use crate::backend::{BackendError, BackendErrorCode, DocumentLocatorDto};
use crate::filesystem::adapter::TauriFilesystemDocumentAdapter;
use crate::library_registry::{LibraryBinding, LibraryBindingRegistry, LibraryBindingRoot};
use crate::mobile_directory_picker::AndroidDirectoryPickerState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryConfigPayload {
    library_id: String,
    #[serde(default)]
    config: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryConfigResult {
    ok: bool,
    /// Normalized configuration; `None` when the library has none yet.
    config: Option<Value>,
    error: Option<String>,
}

impl LibraryConfigResult {
    /// The Google Cloud client, the mail accounts' tokens, the WhatsApp
    /// chats and the webhook actions stay in the backend: clients get the
    /// configuration without them.
    fn from_result(result: Result<Option<Value>, BackendError>) -> Self {
        match result {
            Ok(config) => Self {
                ok: true,
                config: config.map(|config| without_webhook(without_whatsapp(without_google_secrets(config)))),
                error: None,
            },
            Err(error) => Self {
                ok: false,
                config: None,
                error: Some(error.message),
            },
        }
    }
}

struct LibraryConfigStore<'a> {
    app: &'a AppHandle,
    library_id: String,
    binding: LibraryBinding,
    adapter: TauriFilesystemDocumentAdapter<'a>,
    picker: &'a AndroidDirectoryPickerState,
}

impl<'a> LibraryConfigStore<'a> {
    fn open(app: &'a AppHandle, library_id: &str) -> Result<Self, BackendError> {
        let registry = app.state::<LibraryBindingRegistry>().inner();
        let picker = app.state::<AndroidDirectoryPickerState>().inner();
        Ok(Self {
            app,
            library_id: library_id.to_string(),
            binding: registry.lookup(library_id)?,
            adapter: TauriFilesystemDocumentAdapter::for_library(registry, library_id, picker)?,
            picker,
        })
    }

    fn locator(&self) -> Result<DocumentLocatorDto, BackendError> {
        DocumentLocatorDto::new(&self.library_id, LIBRARY_CONFIG_LOGICAL_PATH, None, None)
    }

    fn exists(&self) -> Result<bool, BackendError> {
        match &self.binding.root {
            Some(LibraryBindingRoot::Desktop { canonical_root }) => {
                let _ = self.picker;
                Ok(canonical_root.join(LIBRARY_CONFIG_LOGICAL_PATH).is_file())
            }
            Some(LibraryBindingRoot::Android { tree_uri }) => {
                #[cfg(target_os = "android")]
                {
                    let path = format!("{}/{}", tree_uri.as_str(), LIBRARY_CONFIG_LOGICAL_PATH);
                    Ok(crate::filesystem::android_saf::path_exists(
                        self.picker,
                        &path,
                        Some(tree_uri.as_str()),
                    )
                    .exists)
                }
                #[cfg(not(target_os = "android"))]
                {
                    let _ = tree_uri;
                    Err(unavailable())
                }
            }
            None => Err(unavailable()),
        }
    }

    /// What the file holds, without opening it.
    fn stored(&self) -> Result<Option<StoredConfig>, BackendError> {
        if !self.exists()? {
            return Ok(None);
        }
        classify_stored_config(&self.adapter.read_locator(&self.locator()?)?).map(Some)
    }

    /// The configuration text of an encrypted file, with the unlocked key.
    /// A key that no longer opens it (the file was sealed again elsewhere)
    /// locks the library.
    fn open_envelope(&self, envelope: &ConfigEnvelope) -> Result<String, BackendError> {
        let unlocked = crate::config_vault::unlocked(self.app, &self.library_id).ok_or_else(locked_error)?;
        crate::config_crypto::open_config(&unlocked.key, envelope).ok_or_else(|| {
            crate::config_vault::lock(self.app, &self.library_id);
            locked_error()
        })
    }

    fn read(&self) -> Result<Option<Value>, BackendError> {
        let (normalized, plain) = match self.stored()? {
            None => return Ok(None),
            Some(StoredConfig::Encrypted(envelope)) => (parse_library_config(&self.open_envelope(&envelope)?)?, false),
            Some(StoredConfig::Plain(text)) => (parse_library_config(&text)?, true),
        };
        // A plain file of an unlocked library is sealed now.
        if normalized.needs_migration || plain && crate::config_vault::is_unlocked(self.app, &self.library_id) {
            self.persist(&normalized.config, true)?;
        }
        Ok(Some(normalized.config))
    }

    /// The text to store: sealed while the library is unlocked. The wrapped
    /// key already on disk is kept (the password may have changed on
    /// another device). An encrypted file cannot be written while locked;
    /// a library without an Owner password stays plain.
    fn sealed_text(&self, text: String) -> Result<String, BackendError> {
        let stored = match self.stored()? {
            Some(StoredConfig::Encrypted(envelope)) => Some(envelope),
            _ => None,
        };
        let Some(unlocked) = crate::config_vault::unlocked(self.app, &self.library_id) else {
            return if stored.is_some() { Err(locked_error()) } else { Ok(text) };
        };
        let wrapped = match stored {
            Some(envelope) => {
                self.open_envelope(&envelope)?;
                envelope.key
            }
            None => unlocked.wrapped.clone(),
        };
        serialize_envelope(&crate::config_crypto::seal_config(&unlocked.key, &wrapped, &text)?)
    }

    /// Writes the configuration atomically, creating `.notia/` when needed.
    /// SAF creates the whole relative path in one native call; desktop
    /// creates the directory under the canonical root first.
    fn persist(&self, config: &Value, exists: bool) -> Result<(), BackendError> {
        let text = self.sealed_text(serialize_library_config(config)?)?;
        self.write_text(&text, exists)
    }

    fn write_text(&self, text: &str, exists: bool) -> Result<(), BackendError> {
        let locator = self.locator()?;
        if exists {
            return self.adapter.write_locator(&locator, text, None);
        }
        if let Some(LibraryBindingRoot::Desktop { canonical_root }) = &self.binding.root {
            std::fs::create_dir_all(canonical_root.join(LIBRARY_CONFIG_DIRECTORY)).map_err(|_| {
                BackendError::new(
                    BackendErrorCode::Storage,
                    "No se pudo crear el directorio de configuración.",
                    true,
                )
            })?;
        }
        self.adapter.create_text_locator(&locator, text)
    }
}

/// Normalized configuration of a library for other backend modules (context
/// catalog, provider settings). `None` when the library has none.
/// Changes when Notia writes a library's configuration or a library is
/// locked or unlocked: what keeps a decision taken from a configuration
/// (the Telegram supervisor) takes it again.
static CONFIG_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn config_changed() {
    CONFIG_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn config_generation() -> u64 {
    CONFIG_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

pub(crate) fn read_library_config(app: &AppHandle, library_id: &str) -> Result<Option<Value>, BackendError> {
    LibraryConfigStore::open(app, library_id)?.read()
}

/// What the library's configuration file holds, without opening it.
pub(crate) fn stored_library_config(app: &AppHandle, library_id: &str) -> Result<Option<StoredConfig>, BackendError> {
    LibraryConfigStore::open(app, library_id)?.stored()
}

/// Stores `envelope` as the library's configuration: the same sealed
/// configuration with a new wrapped key (a new Owner password).
pub(crate) fn replace_config_envelope(app: &AppHandle, library_id: &str, envelope: &ConfigEnvelope) -> Result<(), BackendError> {
    let store = LibraryConfigStore::open(app, library_id)?;
    let exists = store.exists()?;
    store.write_text(&serialize_envelope(envelope)?, exists)
}

/// Changes the stored configuration from the backend (for example the mail
/// accounts, which clients cannot write) and returns what was persisted.
pub(crate) fn update_library_config(
    app: &AppHandle,
    library_id: &str,
    change: impl FnOnce(Value) -> Value,
) -> Result<Value, BackendError> {
    let store = LibraryConfigStore::open(app, library_id)?;
    let exists = store.exists()?;
    let stored = if exists { store.read()? } else { None };
    let normalized = normalize_library_config(&change(stored.unwrap_or_else(default_library_config))).config;
    store.persist(&normalized, exists)?;
    Ok(normalized)
}

fn unavailable() -> BackendError {
    BackendError::new(
        BackendErrorCode::Forbidden,
        "La biblioteca no está disponible; volvé a seleccionarla.",
        true,
    )
}

/// Reads the normalized configuration, migrating legacy files in place.
pub(crate) fn backend_read_library_config(payload: LibraryConfigPayload, app: &AppHandle) -> LibraryConfigResult {
    LibraryConfigResult::from_result(LibraryConfigStore::open(app, &payload.library_id).and_then(|store| store.read()))
}

/// Sections only the host changes: a client's write and its offline copy
/// keep the host's value (Telegram is never turned on from a client).
const HOST_ONLY_KEYS: [&str; 5] = [TELEGRAM_KEY, MAIL_ACCOUNTS_KEY, GOOGLE_CLOUD_KEY, WHATSAPP_KEY, WEBHOOK_KEY];
const TELEGRAM_KEY: &str = "telegram";

/// Normalizes and stores the configuration sent by the window and returns
/// what was persisted. `from_client`: a client (of this host, or this
/// device in client mode) sent it, and the Telegram section stays as stored.
pub(crate) fn backend_write_library_config(payload: LibraryConfigPayload, app: &AppHandle, from_client: bool) -> LibraryConfigResult {
    LibraryConfigResult::from_result((|| {
        let config = payload
            .config
            .as_ref()
            .ok_or_else(|| BackendError::invalid_input("Falta la configuración a guardar."))?;
        if !config.is_object() {
            return Err(BackendError::invalid_input("La configuración debe ser un objeto."));
        }
        let names_system = config
            .get("contexts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|context| context.get("tag").and_then(Value::as_str))
            .any(crate::backend::library_config::is_system_context);
        if names_system {
            return Err(BackendError::invalid_input(
                "#System es un contexto reservado de Notia para los archivos que escribe la IA: elegí otro nombre.",
            ));
        }
        let store = LibraryConfigStore::open(app, &payload.library_id)?;
        // Sections the client does not edit (for example the LlamaCloud
        // credential) keep their stored value instead of being erased.
        // A corrupt plain file is replaced; a locked or damaged encrypted
        // one is never overwritten.
        let exists = store.exists()?;
        let stored = if exists {
            match store.read() {
                Ok(config) => config,
                Err(error) if !matches!(store.stored(), Ok(Some(StoredConfig::Plain(_)))) => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        let mut merged = stored.unwrap_or_else(|| Value::Object(Default::default()));
        if let (Some(target), Some(updates)) = (merged.as_object_mut(), config.as_object()) {
            // The Google Cloud client, the mail accounts, the weather place,
            // WhatsApp and the webhook are written only by the backend's own
            // commands.
            let backend_owned = [MAIL_ACCOUNTS_KEY, GOOGLE_CLOUD_KEY, crate::backend::weather::WEATHER_KEY, WHATSAPP_KEY, WEBHOOK_KEY];
            let kept = |key: &str| backend_owned.contains(&key) || (from_client && key == TELEGRAM_KEY);
            for (key, value) in updates.iter().filter(|(key, _)| !kept(key.as_str())) {
                target.insert(key.clone(), value.clone());
            }
        }
        let normalized = normalize_library_config(&merged).config;
        store.persist(&normalized, exists)?;
        Ok(Some(normalized))
    })())
}

/// `config` with the host-only sections of `stored` (removed when the
/// host has none).
fn with_host_sections(mut config: Value, stored: Option<&Value>) -> Value {
    if let Some(target) = config.as_object_mut() {
        for key in HOST_ONLY_KEYS {
            match stored.and_then(|stored| stored.get(key)) {
                Some(value) => target.insert(key.to_string(), value.clone()),
                None => target.remove(key),
            };
        }
    }
    config
}

/// Takes the configuration a client changed on its offline copy (the file
/// as the copy has it, sealed or plain): Telegram, the Google Cloud client
/// and the mail accounts keep the host's value. The library must be
/// unlocked on the host to open a sealed copy.
pub(crate) fn merge_client_config(app: &AppHandle, library_id: &str, text: &str) -> Result<(), BackendError> {
    let store = LibraryConfigStore::open(app, library_id)?;
    let uploaded = match classify_stored_config(text)? {
        StoredConfig::Encrypted(envelope) => {
            let unlocked = crate::config_vault::unlocked(app, library_id).ok_or_else(locked_error)?;
            let text = crate::config_crypto::open_config(&unlocked.key, &envelope).ok_or_else(|| {
                BackendError::new(BackendErrorCode::Forbidden, "El host no pudo abrir la configuración de la copia.", false)
            })?;
            parse_library_config(&text)?.config
        }
        StoredConfig::Plain(text) => parse_library_config(&text)?.config,
    };
    let exists = store.exists()?;
    let stored = if exists { store.read()? } else { None };
    let merged = with_host_sections(uploaded, stored.as_ref());
    store.persist(&normalize_library_config(&merged).config, exists)
}

/// Creates the default configuration when the library has none. An
/// encrypted one is left as is, even locked: adding the folder again must
/// work, and the Owner's sign-in opens it afterwards.
pub(crate) fn backend_ensure_library_config(payload: LibraryConfigPayload, app: &AppHandle) -> LibraryConfigResult {
    LibraryConfigResult::from_result((|| {
        let store = LibraryConfigStore::open(app, &payload.library_id)?;
        if matches!(store.stored()?, Some(StoredConfig::Encrypted(_))) && crate::config_vault::unlocked(app, &payload.library_id).is_none() {
            return Ok(None);
        }
        if let Some(config) = store.read()? {
            return Ok(Some(config));
        }
        let config = default_library_config();
        store.persist(&config, false)?;
        Ok(Some(config))
    })())
}
