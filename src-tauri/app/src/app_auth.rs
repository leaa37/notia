//! Sign-in of the app. Only the Owner opens a library: their password
//! unlocks the library's encrypted configuration (`config_vault`), and
//! while it is locked the interface shows the sign-in window and the
//! background services that need the configuration wait.
//!
//! Flows (the login design): sign in; first time (the Owner has no password
//! yet: the user name, then «Creá tu contraseña»); and change the password
//! (the current one, then the new one). A plain configuration from before
//! the password is sealed at the first sign-in. There is no recovery: a
//! forgotten password leaves the configuration unreadable.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::backend::config_envelope::{ConfigEnvelope, StoredConfig};
use crate::backend::{BackendError, BackendErrorCode};
use crate::config_crypto::{unwrap_key, wrap_key, DataKey};
use crate::config_vault::Unlocked;
use crate::host::{AppHandle, Manager};
use crate::library_catalog::CatalogLibrary;
use crate::library_users::LibraryDatabaseContext;

const MAX_FAILED_ATTEMPTS: u32 = 5;
const FAILED_ATTEMPTS_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AuthState {
    /// No library is selected or reachable: nothing to protect yet.
    None,
    Unlocked,
    Locked,
    /// The Owner has no password yet: the first-time flow creates it.
    Setup,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthStatus {
    state: AuthState,
    library_id: Option<String>,
    library_name: Option<String>,
    /// User name and password remembered on this device («Recordar datos»).
    remembered: Option<RememberedLogin>,
    /// A session is remembered on this device («Recordar sesión»).
    session_remembered: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RememberedLogin {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthLibraryPayload {
    /// The selected library when absent.
    #[serde(default)]
    library_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginPayload {
    library_id: String,
    username: String,
    password: String,
    #[serde(default)]
    remember_session: bool,
    #[serde(default)]
    remember_data: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FirstLoginPayload {
    library_id: String,
    username: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePasswordPayload {
    library_id: String,
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangePasswordPayload {
    library_id: String,
    username: String,
    current: String,
    new: String,
}

/// The answer to a wrong Owner password in ColdPass. The fingerprint unlock
/// recognizes it (also from a host) to forget a password that changed.
pub(crate) const WRONG_OWNER_PASSWORD: &str = "La contraseña del Owner no es correcta.";

fn unauthorized(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Unauthorized, message, false)
}

fn wrong_credentials() -> BackendError {
    unauthorized("Usuario o contraseña incorrectos.")
}

fn data_error(error: crate::library_users::LibraryDataError) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, error.message, true)
}

fn library(app: &AppHandle, library_id: Option<&str>) -> Option<CatalogLibrary> {
    match library_id {
        Some(id) => crate::library_catalog::catalog_library(app, id),
        None => crate::library_catalog::selected_library(app),
    }
}

fn known_library(app: &AppHandle, library_id: &str) -> Result<CatalogLibrary, BackendError> {
    library(app, Some(library_id)).ok_or_else(|| BackendError::new(BackendErrorCode::NotFound, "La biblioteca no existe.", false))
}

fn database_context(library: &CatalogLibrary) -> LibraryDatabaseContext {
    LibraryDatabaseContext { library_path: library.path.clone(), android_directory_uri: library.android_tree_uri.clone() }
}

/// The Owner's stored password hash, after checking that `username` is the
/// Owner: only the Owner opens the app.
fn owner_hash(app: &AppHandle, library: &CatalogLibrary, username: &str) -> Result<Option<String>, BackendError> {
    let (name, hash) = crate::library_users::owner_account(app, &database_context(library)).map_err(data_error)?;
    if !name.trim().eq_ignore_ascii_case(username.trim()) {
        return Err(unauthorized("Solo el usuario Owner puede iniciar sesión en la app."));
    }
    Ok(hash)
}

fn stored_envelope(app: &AppHandle, library_id: &str) -> Result<Option<ConfigEnvelope>, BackendError> {
    Ok(match crate::library_config::stored_library_config(app, library_id)? {
        Some(StoredConfig::Encrypted(envelope)) => Some(envelope),
        _ => None,
    })
}

// ---------- Failed attempts ----------

fn failures() -> &'static Mutex<HashMap<String, (u32, Instant)>> {
    static FAILURES: std::sync::OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = std::sync::OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn check_cooldown(library_id: &str) -> Result<(), BackendError> {
    let Ok(failures) = failures().lock() else { return Ok(()) };
    match failures.get(library_id) {
        Some((count, last)) if *count >= MAX_FAILED_ATTEMPTS && last.elapsed() < FAILED_ATTEMPTS_COOLDOWN => {
            Err(unauthorized("Demasiados intentos fallidos. Esperá 30 segundos y volvé a intentar."))
        }
        _ => Ok(()),
    }
}

fn record_attempt(library_id: &str, ok: bool) {
    let Ok(mut failures) = failures().lock() else { return };
    if ok {
        failures.remove(library_id);
        return;
    }
    let entry = failures.entry(library_id.to_string()).or_insert((0, Instant::now()));
    if entry.1.elapsed() >= FAILED_ATTEMPTS_COOLDOWN && entry.0 >= MAX_FAILED_ATTEMPTS {
        entry.0 = 0;
    }
    entry.0 += 1;
    entry.1 = Instant::now();
}

// ---------- Commands ----------

/// Whether the library is open, locked or waits for the Owner's first
/// password, with what this device remembers.
pub(crate) fn app_auth_status(app: &AppHandle, payload: AuthLibraryPayload) -> Result<AuthStatus, BackendError> {
    let none = || AuthStatus { state: AuthState::None, library_id: None, library_name: None, remembered: None, session_remembered: false };
    let Some(library) = library(app, payload.library_id.as_deref()) else {
        return Ok(none());
    };
    // A library whose folder is not reachable is chosen again first.
    if app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library.id).is_err() {
        return Ok(none());
    }
    let state = if crate::config_vault::is_unlocked(app, &library.id) {
        AuthState::Unlocked
    } else {
        let (_, hash) = crate::library_users::owner_account(app, &database_context(&library)).map_err(data_error)?;
        if hash.is_none() && stored_envelope(app, &library.id)?.is_none() { AuthState::Setup } else { AuthState::Locked }
    };
    Ok(AuthStatus {
        state,
        remembered: crate::config_vault::remembered_credentials(app, &library.id)
            .map(|(username, password)| RememberedLogin { username, password }),
        session_remembered: crate::config_vault::has_remembered_session(app, &library.id),
        library_id: Some(library.id),
        library_name: Some(library.name),
    })
}

/// Signs the Owner in: opens (or, the first time, seals) the configuration.
pub(crate) fn app_auth_login(app: &AppHandle, payload: LoginPayload) -> Result<AuthStatus, BackendError> {
    let library = known_library(app, &payload.library_id)?;
    check_cooldown(&library.id)?;
    let result = sign_in(app, &library, &payload.username, &payload.password);
    record_attempt(&library.id, result.is_ok());
    let unlocked = result?;
    if payload.remember_session {
        crate::config_vault::remember_session(app, &library.id, &unlocked)?;
    } else {
        crate::config_vault::forget_session(app, &library.id);
    }
    if payload.remember_data {
        crate::config_vault::remember_credentials(app, &library.id, payload.username.trim(), &payload.password)?;
    } else {
        crate::config_vault::forget_credentials(app, &library.id);
    }
    app_auth_status(app, AuthLibraryPayload { library_id: Some(library.id) })
}

fn sign_in(app: &AppHandle, library: &CatalogLibrary, username: &str, password: &str) -> Result<Unlocked, BackendError> {
    let hash = owner_hash(app, library, username)?;
    let unlocked = match stored_envelope(app, &library.id)? {
        Some(envelope) => {
            let key = unwrap_key(&envelope.key, password).ok_or_else(wrong_credentials)?;
            // The stored hash follows the password that opens the file.
            if !hash.as_deref().is_some_and(|hash| crate::user_auth::verify_password(password, hash)) {
                crate::library_users::set_owner_password(app, &database_context(library), password).map_err(data_error)?;
            }
            Unlocked { key, wrapped: envelope.key }
        }
        None => {
            let hash = hash.ok_or_else(|| unauthorized("El Owner todavía no tiene contraseña: usá «Primer inicio»."))?;
            if !crate::user_auth::verify_password(password, &hash) {
                return Err(wrong_credentials());
            }
            let key = DataKey::new()?;
            let wrapped = wrap_key(&key, password)?;
            Unlocked { key, wrapped }
        }
    };
    crate::config_vault::unlock(app, &library.id, unlocked.clone());
    // A plain configuration is sealed by reading it once unlocked.
    if let Err(error) = crate::library_config::read_library_config(app, &library.id) {
        crate::config_vault::lock(app, &library.id);
        return Err(error);
    }
    Ok(unlocked)
}

/// First step of «Primer inicio»: the user name must be the Owner's, and the
/// Owner must not have a password yet.
pub(crate) fn app_auth_first_login(app: &AppHandle, payload: FirstLoginPayload) -> Result<(), BackendError> {
    let library = known_library(app, &payload.library_id)?;
    let hash = owner_hash(app, &library, &payload.username)?;
    if hash.is_some() || stored_envelope(app, &library.id)?.is_some() {
        return Err(unauthorized("Este usuario ya tiene contraseña: iniciá sesión."));
    }
    Ok(())
}

/// «Creá tu contraseña»: stores the Owner's first password and seals the
/// configuration with it. The person then signs in.
pub(crate) fn app_auth_create_password(app: &AppHandle, payload: CreatePasswordPayload) -> Result<AuthStatus, BackendError> {
    app_auth_first_login(app, FirstLoginPayload { library_id: payload.library_id.clone(), username: payload.username.clone() })?;
    crate::user_auth::validate_password(&payload.password).map_err(|message| BackendError::invalid_input(message))?;
    let library = known_library(app, &payload.library_id)?;
    crate::library_users::set_owner_password(app, &database_context(&library), &payload.password).map_err(data_error)?;
    let key = DataKey::new()?;
    let wrapped = wrap_key(&key, &payload.password)?;
    crate::config_vault::unlock(app, &library.id, Unlocked { key, wrapped });
    let sealed = crate::library_config::read_library_config(app, &library.id);
    crate::config_vault::lock(app, &library.id);
    sealed?;
    app_auth_status(app, AuthLibraryPayload { library_id: Some(library.id) })
}

/// «Cambiar contraseña»: checks the current password and seals the data key
/// with the new one; the configuration itself does not change.
pub(crate) fn app_auth_change_password(app: &AppHandle, payload: ChangePasswordPayload) -> Result<AuthStatus, BackendError> {
    let library = known_library(app, &payload.library_id)?;
    crate::user_auth::validate_password(&payload.new).map_err(|message| BackendError::invalid_input(message))?;
    if payload.new == payload.current {
        return Err(BackendError::invalid_input("La nueva contraseña tiene que ser distinta de la actual."));
    }
    check_cooldown(&library.id)?;
    let result = change_password(app, &library, &payload);
    record_attempt(&library.id, result.is_ok());
    result?;
    app_auth_status(app, AuthLibraryPayload { library_id: Some(library.id) })
}

fn change_password(app: &AppHandle, library: &CatalogLibrary, payload: &ChangePasswordPayload) -> Result<(), BackendError> {
    let hash = owner_hash(app, library, &payload.username)?;
    match stored_envelope(app, &library.id)? {
        Some(envelope) => {
            let key = unwrap_key(&envelope.key, &payload.current).ok_or_else(wrong_credentials)?;
            let wrapped = wrap_key(&key, &payload.new)?;
            crate::library_config::replace_config_envelope(app, &library.id, &ConfigEnvelope::new(wrapped.clone(), envelope.payload))?;
            crate::config_vault::update_wrapped(app, &library.id, &wrapped);
        }
        None => {
            let hash = hash.ok_or_else(|| unauthorized("El Owner todavía no tiene contraseña: usá «Primer inicio»."))?;
            if !crate::user_auth::verify_password(&payload.current, &hash) {
                return Err(wrong_credentials());
            }
        }
    }
    crate::library_users::set_owner_password(app, &database_context(library), &payload.new).map_err(data_error)?;
    after_owner_password_changed(app, &library.id, &payload.new);
    Ok(())
}

/// Remembered credentials follow the new password, and the Owner's
/// published Task Manager sessions end.
fn after_owner_password_changed(app: &AppHandle, library_id: &str, password: &str) {
    if let Some((username, _)) = crate::config_vault::remembered_credentials(app, library_id) {
        let _ = crate::config_vault::remember_credentials(app, library_id, &username, password);
    }
    crate::task_manager_publication::revoke_library_user_sessions(
        app.state::<crate::task_manager_publication::TaskManagerPublicationState>().inner(),
        "user-owner",
    );
}

/// Signs out of the library: its key leaves memory and this device, and
/// ColdPass locks.
pub(crate) fn app_auth_logout(app: &AppHandle, payload: AuthLibraryPayload) -> Result<AuthStatus, BackendError> {
    if let Some(library) = library(app, payload.library_id.as_deref()) {
        crate::config_vault::lock(app, &library.id);
        app.state::<crate::coldpass::ColdPassState>().lock_library(&library.id);
    }
    app_auth_status(app, payload)
}

/// The data key of the library's configuration, opened with the Owner's
/// password as it is stored now. ColdPass derives its vault key from it, so
/// it asks for the same password and follows its changes. Failed attempts
/// share the sign-in cooldown.
pub(crate) fn owner_data_key(app: &AppHandle, library_id: &str, password: &str) -> Result<DataKey, BackendError> {
    let library = known_library(app, library_id)?;
    check_cooldown(&library.id)?;
    let result = stored_envelope(app, &library.id).and_then(|envelope| {
        let envelope = envelope.ok_or_else(|| unauthorized("Iniciá sesión con el Owner antes de usar ColdPass."))?;
        unwrap_key(&envelope.key, password).ok_or_else(|| unauthorized(WRONG_OWNER_PASSWORD))
    });
    record_attempt(&library.id, result.is_ok());
    result
}

/// Seals the configuration of the library at `library_path` with the
/// Owner's new password set from Settings, while the library is unlocked.
/// Called before the password is stored, so a locked library keeps both.
pub(crate) fn owner_password_set_in_settings(
    app: &AppHandle,
    context: &LibraryDatabaseContext,
    password: &str,
) -> Result<(), BackendError> {
    let Some(library) = crate::library_catalog::catalog_libraries(app)
        .into_iter()
        .find(|library| library.path == context.library_path)
    else {
        return Ok(());
    };
    let Some(envelope) = stored_envelope(app, &library.id)? else {
        return Ok(());
    };
    let unlocked = crate::config_vault::unlocked(app, &library.id)
        .ok_or_else(|| unauthorized("Iniciá sesión con el Owner para cambiar su contraseña."))?;
    let wrapped = wrap_key(&unlocked.key, password)?;
    crate::library_config::replace_config_envelope(app, &library.id, &ConfigEnvelope::new(wrapped.clone(), envelope.payload))?;
    crate::config_vault::update_wrapped(app, &library.id, &wrapped);
    after_owner_password_changed(app, &library.id, password);
    Ok(())
}

// ---------- The Host mode: the library this installation serves (desktop) ----------

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn host_library(app: &AppHandle) -> Result<CatalogLibrary, BackendError> {
    crate::library_catalog::selected_library(app)
        .ok_or_else(|| BackendError::new(BackendErrorCode::NotFound, "El host no tiene una biblioteca abierta.", true))
}

/// Status of the served library for a client's sign-in window. What this
/// device remembers stays here.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn host_status(app: &AppHandle) -> Result<serde_json::Value, BackendError> {
    let mut status = app_auth_status(app, AuthLibraryPayload { library_id: None })?;
    status.remembered = None;
    status.session_remembered = false;
    serde_json::to_value(status).map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo leer el estado.", false))
}

/// A client signs in with the library's Owner: the same checks as the app,
/// which also unlock the library on this host.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn host_sign_in(app: &AppHandle, username: &str, password: &str) -> Result<(), BackendError> {
    let library = host_library(app)?;
    check_cooldown(&library.id)?;
    let result = sign_in(app, &library, username, password).map(|_| ());
    record_attempt(&library.id, result.is_ok());
    result
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn host_first_login(app: &AppHandle, username: &str) -> Result<(), BackendError> {
    app_auth_first_login(app, FirstLoginPayload { library_id: host_library(app)?.id, username: username.to_string() })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn host_create_password(app: &AppHandle, username: &str, password: &str) -> Result<(), BackendError> {
    let payload = CreatePasswordPayload { library_id: host_library(app)?.id, username: username.to_string(), password: password.to_string() };
    app_auth_create_password(app, payload).map(|_| ())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn host_change_password(app: &AppHandle, username: &str, current: &str, new: &str) -> Result<(), BackendError> {
    let payload = ChangePasswordPayload {
        library_id: host_library(app)?.id,
        username: username.to_string(),
        current: current.to_string(),
        new: new.to_string(),
    };
    app_auth_change_password(app, payload).map(|_| ())
}

// ---------- Async commands (key derivation takes a moment) ----------

async fn blocking<T: Send + 'static>(run: impl FnOnce() -> Result<T, BackendError> + Send + 'static) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(run)
        .await
        .map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo completar el inicio de sesión.", true))?
}

fn to_value(status: AuthStatus) -> Result<serde_json::Value, BackendError> {
    serde_json::to_value(status).map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo leer el estado.", false))
}

// A client signs in on its copy, which is the library it works on: the
// Owner's user name and password also open the session of the sync with
// the host (`host_client::use_credentials`). Only the first copy, before
// there is a copy to open, signs in with the host itself.

/// Whether this device is a client still without its copy open: it signs
/// in with the host for the first copy.
fn signs_in_with_host(app: &AppHandle) -> bool {
    crate::connection::is_client(app) && !crate::host_mirror::is_open(app)
}

fn host_signs_in_first() -> BackendError {
    BackendError::new(
        BackendErrorCode::Unsupported,
        "Para copiar la biblioteca, el Owner tiene que tener contraseña: creala en el host.",
        false,
    )
}

/// Signs the Owner in on the copy right after the first copy, with the
/// session that copied it: the person does not sign in twice.
pub(crate) fn unlock_copy(app: &AppHandle, library_id: &str, username: &str, password: &str, remember_session: bool) -> Result<(), BackendError> {
    if crate::config_vault::is_unlocked(app, library_id) {
        return Ok(());
    }
    let payload = LoginPayload {
        library_id: library_id.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        remember_session,
        remember_data: false,
    };
    app_auth_login(app, payload).map(|_| ())
}

pub(crate) async fn backend_app_auth_status(app: AppHandle, payload: AuthLibraryPayload) -> Result<serde_json::Value, BackendError> {
    let status = if signs_in_with_host(&app) {
        crate::host_client::auth_status(&app).await
    } else {
        blocking(move || app_auth_status(&app, payload).and_then(to_value)).await
    };
    // Without it the window cannot open the library: the reason goes to the log.
    if let Err(error) = &status {
        log::error!("[notia:auth] the sign-in status failed: {}", error.message);
    }
    status
}

/// On a client, the copy keeps the password the host had at the last sync:
/// when it changed on another device, the host accepts the new one, and a
/// sync brings it to the copy before signing in again.
pub(crate) async fn backend_app_auth_login(app: AppHandle, payload: LoginPayload) -> Result<serde_json::Value, BackendError> {
    if signs_in_with_host(&app) {
        let LoginPayload { library_id, username, password, remember_session, remember_data } = payload;
        return crate::host_client::auth_login(&app, &library_id, &username, &password, remember_session, remember_data).await;
    }
    let client = crate::connection::is_client(&app);
    let LoginPayload { library_id, username, password, remember_session, remember_data } = payload;
    let attempt = |app: AppHandle| {
        let payload = LoginPayload {
            library_id: library_id.clone(),
            username: username.clone(),
            password: password.clone(),
            remember_session,
            remember_data,
        };
        blocking(move || app_auth_login(&app, payload).and_then(to_value))
    };
    let mut signed_in = attempt(app.clone()).await;
    if client && signed_in.as_ref().is_err_and(|error| error.code == BackendErrorCode::Unauthorized)
        && crate::host_client::host_accepts(&app, &username, &password).await
    {
        crate::host_mirror::sync(&app).await?;
        signed_in = attempt(app.clone()).await;
    }
    let status = signed_in?;
    if client {
        crate::host_client::use_credentials(&app, &username, &password, remember_session);
    }
    Ok(status)
}

pub(crate) async fn backend_app_auth_first_login(app: AppHandle, payload: FirstLoginPayload) -> Result<(), BackendError> {
    if signs_in_with_host(&app) {
        return Err(host_signs_in_first());
    }
    blocking(move || app_auth_first_login(&app, payload)).await
}

pub(crate) async fn backend_app_auth_create_password(app: AppHandle, payload: CreatePasswordPayload) -> Result<serde_json::Value, BackendError> {
    if signs_in_with_host(&app) {
        return Err(host_signs_in_first());
    }
    blocking(move || app_auth_create_password(&app, payload).and_then(to_value)).await
}

/// On a client the host changes first, so the copy and the host never
/// disagree on the password; without connection it waits.
pub(crate) async fn backend_app_auth_change_password(app: AppHandle, payload: ChangePasswordPayload) -> Result<serde_json::Value, BackendError> {
    if crate::connection::is_client(&app) {
        crate::host_client::change_password(&app, &payload.username, &payload.current, &payload.new)
            .await
            .map_err(|error| match error.code {
                BackendErrorCode::ProviderUnavailable => BackendError::new(
                    BackendErrorCode::ProviderUnavailable,
                    "Para cambiar la contraseña hace falta conexión con el host: probá de nuevo cuando responda.",
                    true,
                ),
                _ => error,
            })?;
        // Before the first copy there is no copy to change.
        if signs_in_with_host(&app) {
            return crate::host_client::auth_status(&app).await;
        }
    }
    blocking(move || app_auth_change_password(&app, payload).and_then(to_value)).await
}

pub(crate) async fn backend_app_auth_logout(app: AppHandle, payload: AuthLibraryPayload) -> Result<serde_json::Value, BackendError> {
    if crate::connection::is_client(&app) {
        crate::host_client::sign_out(&app);
    }
    blocking(move || app_auth_logout(&app, payload).and_then(to_value)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_failures_wait_before_another_try() {
        let library = "library-attempts";
        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert!(check_cooldown(library).is_ok());
            record_attempt(library, false);
        }
        assert!(check_cooldown(library).is_err());
        record_attempt(library, true);
        assert!(check_cooldown(library).is_ok());
    }

    #[test]
    fn the_owner_seals_opens_and_seals_again_the_configuration() {
        use crate::host::{AppPaths, HostPorts};
        use crate::library_config::{read_library_config, update_library_config};
        let root = std::env::temp_dir().join(format!("notia-auth-test-{}", uuid::Uuid::new_v4()));
        let folder = root.join("Biblioteca");
        std::fs::create_dir_all(&folder).expect("library folder");
        let app = crate::create_app(AppPaths::new(Some(root.join("data")), None), HostPorts::default());
        // The window's plugin registers it; the test does.
        app.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        let id = crate::library_catalog::add_desktop_library(&app, &folder).expect("library").id;
        let file = folder.join(".notia").join("notiaConfig.json");
        let with_token = |token: &'static str| {
            move |mut config: serde_json::Value| {
                config["telegram"] = serde_json::json!({ "enabled": true, "botToken": token });
                config
            }
        };

        // Before the Owner's password the file is plain.
        update_library_config(&app, &id, with_token("1:secreto")).expect("plain config");
        assert!(std::fs::read_to_string(&file).expect("file").contains("1:secreto"));
        let status = app_auth_status(&app, AuthLibraryPayload { library_id: None }).expect("status");
        assert_eq!(status.state, AuthState::Setup);
        assert!(app_auth_first_login(&app, FirstLoginPayload { library_id: id.clone(), username: "Intruso".into() }).is_err());

        // The first password seals it; locked, nothing reads it.
        let create = CreatePasswordPayload { library_id: id.clone(), username: "owner".into(), password: "contraseña-1".into() };
        assert_eq!(app_auth_create_password(&app, create).expect("created").state, AuthState::Locked);
        let sealed = std::fs::read_to_string(&file).expect("file");
        assert!(sealed.contains("notiaEncrypted") && !sealed.contains("1:secreto"));
        assert!(read_library_config(&app, &id).is_err());
        assert!(update_library_config(&app, &id, with_token("x:x")).is_err());
        // Adding the folder again keeps the sealed file and does not fail:
        // the Owner's sign-in opens it.
        let payload = serde_json::from_value(serde_json::json!({ "libraryId": id })).expect("payload");
        let ensured = serde_json::to_value(crate::library_config::backend_ensure_library_config(payload, &app)).expect("json");
        assert_eq!(ensured["ok"], true, "{ensured}");
        assert_eq!(std::fs::read_to_string(&file).expect("file"), sealed);

        let login = |password: &str| {
            app_auth_login(
                &app,
                LoginPayload {
                    library_id: id.clone(),
                    username: "Owner".into(),
                    password: password.into(),
                    remember_session: false,
                    remember_data: false,
                },
            )
        };
        assert!(login("otra-contraseña").is_err());
        assert_eq!(login("contraseña-1").expect("signed in").state, AuthState::Unlocked);
        let config = read_library_config(&app, &id).expect("read").expect("config");
        assert_eq!(config["telegram"]["botToken"], "1:secreto");

        // Changes stay sealed.
        update_library_config(&app, &id, with_token("2:secreto")).expect("sealed write");
        assert!(!std::fs::read_to_string(&file).expect("file").contains("2:secreto"));

        // A new password seals the same configuration again.
        assert_eq!(app_auth_logout(&app, AuthLibraryPayload { library_id: Some(id.clone()) }).expect("logout").state, AuthState::Locked);
        let change = ChangePasswordPayload {
            library_id: id.clone(),
            username: "owner".into(),
            current: "contraseña-1".into(),
            new: "contraseña-2".into(),
        };
        app_auth_change_password(&app, change).expect("changed");
        record_attempt(&id, true);
        assert!(login("contraseña-1").is_err());
        assert_eq!(login("contraseña-2").expect("signed in").state, AuthState::Unlocked);
        assert_eq!(read_library_config(&app, &id).expect("read").expect("config")["telegram"]["botToken"], "2:secreto");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_status_is_sent_in_kebab_case() {
        assert_eq!(serde_json::to_value(AuthState::Setup).expect("json"), "setup");
        assert_eq!(serde_json::to_value(AuthState::Unlocked).expect("json"), "unlocked");
    }
}
