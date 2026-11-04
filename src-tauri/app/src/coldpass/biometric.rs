//! Fingerprint unlock of ColdPass on this device (Android with a strong
//! fingerprint sensor). The fingerprint stands in for typing the Owner's
//! password: turning it on checks the password by unlocking with it, then
//! seals it with a Keystore key that only works right after the fingerprint.
//! The sealed copy stays in this device's private data folder, never in the
//! library that syncs. Unlocking opens that copy with the fingerprint and
//! runs the usual unlock with it, here or on the host when this device is a
//! client, so the vault key never leaves the backend that holds the vault.
//! When the fingerprints of the device change, Android discards the key;
//! when the Owner's password changes, the sealed one stops working. Either
//! way the fingerprint turns itself off and the password is asked again.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{unlock_with_password, ColdPassPayload};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Manager};
use crate::mobile_biometric::{self, BiometricAvailability, BiometricFailure, BiometricState, PromptText, SealedSecret};

const DIRECTORY: &str = "coldpass-biometric";
const ALIAS_PREFIX: &str = "notia-coldpass-";

const ENABLE_PROMPT: PromptText<'static> = PromptText {
    title: "Activar huella en ColdPass",
    subtitle: "Apoyá el dedo para abrir ColdPass con tu huella en este dispositivo.",
};
const UNLOCK_PROMPT: PromptText<'static> = PromptText {
    title: "Desbloquear ColdPass",
    subtitle: "Apoyá el dedo en el sensor.",
};

/// The Owner's password sealed behind the fingerprint, as stored here.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredUnlock {
    iv: String,
    ciphertext: String,
}

/// What ColdPass can do with the fingerprint on this device.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColdPassBiometricDto {
    /// `available`, `not-enrolled` (no fingerprint registered in Android) or
    /// `unsupported`.
    availability: &'static str,
    /// The fingerprint opens this library's vault on this device.
    enabled: bool,
}

/// The library id reduced to the characters a file name and a Keystore
/// alias accept.
fn library_key(library_id: &str) -> Result<String, BackendError> {
    let key = library_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect::<String>();
    if key.is_empty() {
        Err(BackendError::invalid_input("La biblioteca no es válida."))
    } else {
        Ok(key)
    }
}

fn alias(library_key: &str) -> String {
    format!("{ALIAS_PREFIX}{library_key}")
}

fn file_path(app: &AppHandle, library_key: &str) -> Result<PathBuf, BackendError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(DIRECTORY).join(format!("{library_key}.json")))
        .map_err(|_| storage_error())
}

fn storage_error() -> BackendError {
    BackendError::new(BackendErrorCode::Storage, "No se pudo guardar la huella en este dispositivo.", true)
}

fn read_stored(app: &AppHandle, library_key: &str) -> Option<SealedSecret> {
    let text = std::fs::read_to_string(file_path(app, library_key).ok()?).ok()?;
    let stored: StoredUnlock = serde_json::from_str(&text).ok()?;
    Some(SealedSecret { iv: stored.iv, ciphertext: stored.ciphertext })
}

fn write_stored(app: &AppHandle, library_key: &str, sealed: SealedSecret) -> Result<(), BackendError> {
    let path = file_path(app, library_key)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| storage_error())?;
    }
    let text = serde_json::to_string(&StoredUnlock { iv: sealed.iv, ciphertext: sealed.ciphertext })
        .map_err(|_| storage_error())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|_| storage_error())?;
    std::fs::rename(&temporary, &path).map_err(|_| storage_error())
}

/// Turns the fingerprint off for the library on this device.
fn forget(app: &AppHandle, library_key: &str) {
    if let Ok(path) = file_path(app, library_key) {
        let _ = std::fs::remove_file(path);
    }
    mobile_biometric::remove(app.state::<BiometricState>().inner(), &alias(library_key));
}

fn availability_name(availability: BiometricAvailability) -> &'static str {
    match availability {
        BiometricAvailability::Available => "available",
        BiometricAvailability::NotEnrolled => "not-enrolled",
        BiometricAvailability::Unsupported => "unsupported",
    }
}

fn status(app: &AppHandle, library_key: &str) -> ColdPassBiometricDto {
    let availability = mobile_biometric::availability(app.state::<BiometricState>().inner());
    ColdPassBiometricDto {
        availability: availability_name(availability),
        enabled: availability != BiometricAvailability::Unsupported && read_stored(app, library_key).is_some(),
    }
}

fn require_available(app: &AppHandle) -> Result<(), BackendError> {
    match mobile_biometric::availability(app.state::<BiometricState>().inner()) {
        BiometricAvailability::Available => Ok(()),
        BiometricAvailability::NotEnrolled => Err(BackendError::new(
            BackendErrorCode::Unsupported,
            "Registrá una huella en los ajustes de Android para usarla en ColdPass.",
            true,
        )),
        BiometricAvailability::Unsupported => Err(BackendError::new(
            BackendErrorCode::Unsupported,
            "Este dispositivo no tiene un sensor de huella compatible.",
            false,
        )),
    }
}

fn changed_fingerprints() -> BackendError {
    BackendError::new(
        BackendErrorCode::Forbidden,
        "Cambiaron las huellas de este dispositivo: desbloqueá con la contraseña del Owner y volvé a activar la huella.",
        false,
    )
}

fn changed_password() -> BackendError {
    BackendError::new(
        BackendErrorCode::Forbidden,
        "La contraseña del Owner cambió: desbloqueá con la nueva y volvé a activar la huella.",
        false,
    )
}

fn failure_error(failure: BiometricFailure) -> BackendError {
    match failure {
        BiometricFailure::Cancelled => BackendError::new(BackendErrorCode::Cancelled, "Se canceló la huella.", true),
        BiometricFailure::Lockout => BackendError::new(
            BackendErrorCode::Forbidden,
            "Demasiados intentos con la huella. Usá la contraseña del Owner.",
            true,
        ),
        BiometricFailure::Invalidated | BiometricFailure::Missing => changed_fingerprints(),
        BiometricFailure::Busy => BackendError::new(BackendErrorCode::Conflict, "Ya hay una solicitud de huella abierta.", true),
        BiometricFailure::Unavailable => BackendError::new(
            BackendErrorCode::Unsupported,
            "La huella no está disponible en este dispositivo.",
            true,
        ),
        BiometricFailure::Failed => BackendError::new(BackendErrorCode::Internal, "No se pudo usar la huella.", true),
    }
}

/// The unlock rejected the password itself (not a cooldown or a locked
/// library): the sealed password is not the Owner's any more.
fn is_wrong_password(error: &BackendError) -> bool {
    error.code == BackendErrorCode::Unauthorized && error.message == crate::app_auth::WRONG_OWNER_PASSWORD
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, BackendError> + Send + 'static,
) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(run)
        .await
        .map_err(|_| BackendError::new(BackendErrorCode::Internal, "La huella se interrumpió: volvé a intentarlo.", true))?
}

/// `coldpass_unlock` with `password` on the vault of this device's library
/// (a client's copy too). Returns its `{ entries }`.
async fn unlock_where_the_vault_is(app: &AppHandle, library_id: &str, password: String) -> Result<Value, BackendError> {
    let app = app.clone();
    let payload = ColdPassPayload {
        library_id: library_id.to_string(),
        password: Some(password),
        legacy_passkey: None,
        entry: None,
        entry_id: None,
    };
    let entries = blocking(move || unlock_with_password(&app, payload)).await?;
    serde_json::to_value(entries).map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo abrir ColdPass.", true))
}

/// Whether this device can use the fingerprint and whether it is on for
/// the library.
pub(crate) async fn coldpass_biometric_status(app: AppHandle, payload: ColdPassPayload) -> Result<ColdPassBiometricDto, BackendError> {
    blocking(move || {
        let key = library_key(&payload.library_id)?;
        Ok(status(&app, &key))
    })
    .await
}

/// Turns the fingerprint on: the Owner's password must unlock the vault,
/// then the fingerprint seals it on this device.
pub(crate) async fn coldpass_enable_biometric(app: AppHandle, payload: ColdPassPayload) -> Result<ColdPassBiometricDto, BackendError> {
    let key = library_key(&payload.library_id)?;
    let password = payload
        .password
        .filter(|password| !password.is_empty())
        .ok_or_else(|| BackendError::invalid_input("Ingresá la contraseña del Owner."))?;
    let check = app.clone();
    blocking(move || require_available(&check)).await?;
    unlock_where_the_vault_is(&app, &payload.library_id, password.clone()).await?;
    blocking(move || {
        let sealed = mobile_biometric::seal(app.state::<BiometricState>().inner(), &alias(&key), password.as_bytes(), ENABLE_PROMPT)
            .map_err(|failure| {
                forget(&app, &key);
                failure_error(failure)
            })?;
        write_stored(&app, &key, sealed).inspect_err(|_| forget(&app, &key))?;
        Ok(status(&app, &key))
    })
    .await
}

/// Turns the fingerprint off for the library on this device.
pub(crate) async fn coldpass_disable_biometric(app: AppHandle, payload: ColdPassPayload) -> Result<ColdPassBiometricDto, BackendError> {
    blocking(move || {
        let key = library_key(&payload.library_id)?;
        forget(&app, &key);
        Ok(status(&app, &key))
    })
    .await
}

/// Opens the vault with the fingerprint: the sealed password unlocks it as
/// if it were typed. Returns the same `{ entries }` as `coldpass_unlock`.
pub(crate) async fn coldpass_unlock_biometric(app: AppHandle, payload: ColdPassPayload) -> Result<Value, BackendError> {
    let key = library_key(&payload.library_id)?;
    let device = app.clone();
    let device_key = key.clone();
    let password = blocking(move || {
        let sealed = read_stored(&device, &device_key).ok_or_else(|| {
            BackendError::new(BackendErrorCode::NotFound, "La huella no está activada para ColdPass en este dispositivo.", false)
        })?;
        let bytes = mobile_biometric::open(device.state::<BiometricState>().inner(), &alias(&device_key), &sealed, UNLOCK_PROMPT)
            .map_err(|failure| {
                if matches!(failure, BiometricFailure::Invalidated | BiometricFailure::Missing) {
                    forget(&device, &device_key);
                }
                failure_error(failure)
            })?;
        String::from_utf8(bytes).map_err(|_| {
            forget(&device, &device_key);
            changed_fingerprints()
        })
    })
    .await?;
    unlock_where_the_vault_is(&app, &payload.library_id, password).await.map_err(|error| {
        if is_wrong_password(&error) {
            forget(&app, &key);
            changed_password()
        } else {
            error
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_id_becomes_a_safe_alias() {
        assert_eq!(library_key("lib-1/../x").expect("key"), "lib-1x");
        assert_eq!(alias("lib-1"), "notia-coldpass-lib-1");
        assert!(library_key("../").is_err());
        assert_eq!(library_key(&"a".repeat(200)).expect("key").len(), 64);
    }

    #[test]
    fn availability_is_sent_in_kebab_case() {
        assert_eq!(availability_name(BiometricAvailability::NotEnrolled), "not-enrolled");
        let dto = ColdPassBiometricDto { availability: "available", enabled: true };
        assert_eq!(serde_json::to_value(dto).expect("json"), serde_json::json!({ "availability": "available", "enabled": true }));
    }

    #[test]
    fn a_cancelled_fingerprint_is_not_an_error_to_show() {
        assert_eq!(failure_error(BiometricFailure::Cancelled).code, BackendErrorCode::Cancelled);
        assert_eq!(failure_error(BiometricFailure::Invalidated).code, BackendErrorCode::Forbidden);
        assert!(!failure_error(BiometricFailure::Missing).retryable);
    }

    #[test]
    fn only_a_rejected_password_turns_the_fingerprint_off() {
        let wrong = BackendError::new(BackendErrorCode::Unauthorized, crate::app_auth::WRONG_OWNER_PASSWORD, false);
        assert!(is_wrong_password(&wrong));
        // The error travels from a host as JSON and keeps its meaning.
        let from_host: BackendError = serde_json::from_value(serde_json::to_value(&wrong).expect("json")).expect("error");
        assert!(is_wrong_password(&from_host));
        let cooldown = BackendError::new(BackendErrorCode::Unauthorized, "Demasiados intentos fallidos. Esperá 30 segundos y volvé a intentar.", false);
        assert!(!is_wrong_password(&cooldown));
        let locked = BackendError::new(BackendErrorCode::Unauthorized, "Iniciá sesión con el Owner antes de usar ColdPass.", false);
        assert!(!is_wrong_password(&locked));
    }
}
