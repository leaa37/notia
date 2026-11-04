//! How this installation runs, Host or Client (`backend::connection`), kept
//! per device in the app data folder, and the commands of the «Modo de
//! ejecución» section of Settings.
//!
//! - **Host** (Windows, Linux and macOS): the server of `host_server`
//!   listens on the configured port and the devices that sync with it reach
//!   the library there. On Android, Host only means «uses its own library»:
//!   Android has no server.
//! - **Client**: the window always works on the copy of the host's library
//!   (`host_mirror`), as a library of this device; `host_client` syncs it
//!   whenever the host answers.
//!
//! Which computer runs the library's services is decided apart
//! (`device_role`). A change of mode or host reloads the interface.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::backend::connection::{normalize_connection, parse_host_address, ConnectionSettings, HostAddress, RunMode};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Manager};

const FILE_NAME: &str = "connection.json";

#[derive(Default)]
pub(crate) struct ConnectionState {
    settings: Mutex<Option<ConnectionSettings>>,
}

fn file_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|directory| directory.join(FILE_NAME))
}

fn read_file(app: &AppHandle) -> ConnectionSettings {
    let settings = file_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<ConnectionSettings>(&text).ok())
        .map(normalize_connection)
        .unwrap_or_default();
    // A client without a host cannot start: it runs as Host until one is
    // saved. On Android neither can one without the folder of its copy (a
    // client from before every client kept a copy).
    let without_copy_folder = cfg!(target_os = "android") && settings.copy_folder.is_none();
    if settings.mode == RunMode::Client && (settings.host_address.is_empty() || without_copy_folder) {
        return ConnectionSettings { mode: RunMode::Host, ..settings };
    }
    settings
}

/// The settings of this device. The id of the installation is made the
/// first time and kept with them.
pub(crate) fn settings(app: &AppHandle) -> ConnectionSettings {
    let Some(state) = app.try_state::<ConnectionState>() else {
        return ConnectionSettings::default();
    };
    let mut cached = state.settings.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(settings) = cached.as_ref() {
        return settings.clone();
    }
    let mut settings = read_file(app);
    if settings.device_id.is_empty() {
        settings.device_id = uuid::Uuid::new_v4().to_string();
        if let Err(error) = write_file(app, &settings) {
            log::error!("[notia:connection] the id of this device was not saved: {}", error.message);
        }
    }
    *cached = Some(settings.clone());
    settings
}

fn storage_error() -> BackendError {
    BackendError::new(BackendErrorCode::Storage, "No se pudo guardar el modo de ejecución en este equipo.", true)
}

fn write_file(app: &AppHandle, settings: &ConnectionSettings) -> Result<(), BackendError> {
    let path = file_path(app).ok_or_else(storage_error)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| storage_error())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|_| storage_error())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|_| storage_error())?;
    std::fs::rename(&temporary, &path).map_err(|_| storage_error())
}

/// Stores the settings of this device.
pub(crate) fn store(app: &AppHandle, settings: &ConnectionSettings) -> Result<(), BackendError> {
    write_file(app, settings)?;
    if let Some(state) = app.try_state::<ConnectionState>() {
        *state.settings.lock().unwrap_or_else(|error| error.into_inner()) = Some(settings.clone());
    }
    Ok(())
}

/// The host this device syncs with, and the certificate it trusts, when it
/// runs as a client.
pub(crate) fn client_host(app: &AppHandle) -> Option<(HostAddress, Option<String>)> {
    let settings = settings(app);
    if settings.mode != RunMode::Client {
        return None;
    }
    let address = parse_host_address(&settings.host_address).ok()?;
    Some((address, settings.host_certificate))
}

/// Whether this device runs as a client: it works on its copy of a host's
/// library and syncs it with the host.
pub(crate) fn is_client(app: &AppHandle) -> bool {
    client_host(app).is_some()
}

/// Pins the certificate of the host seen on the first connection, while
/// the address is still the one saved.
pub(crate) fn pin_host_certificate(app: &AppHandle, address: &HostAddress, fingerprint: &str) {
    let mut settings = settings(app);
    if settings.mode != RunMode::Client || settings.host_address != address.display() || settings.host_certificate.is_some() {
        return;
    }
    settings.host_certificate = Some(fingerprint.to_string());
    if let Err(error) = store(app, &settings) {
        log::error!("[notia:connection] could not pin the host certificate: {}", error.message);
    }
}

/// Starts what the mode of this device needs: the server of a host or the
/// link of a client. Called once the window's application is built.
pub(crate) fn start(app: &AppHandle) {
    crate::device_role::runs_services(app);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    crate::host_server::reconfigure(app);
    crate::host_client::reconfigure(app);
    crate::client_status::refresh(app);
}

// ---------- Commands ----------

/// How far the running sync of the copy went.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyProgress {
    done: usize,
    total: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionView {
    mode: RunMode,
    host_address: String,
    /// Whether this device can serve its library (not on Android).
    can_host: bool,
    server: crate::host_client::ServerView,
    link: crate::host_client::LinkView,
    /// Android: the copy needs a folder chosen by the person.
    copy_needs_folder: bool,
    /// Name of the chosen folder of the copy (Android).
    copy_folder: Option<String>,
    /// The copy is ready: the window opens on it.
    copy_ready: bool,
    /// The last sync of the copy.
    copy: Option<crate::host_mirror::CopyStatus>,
    /// The sync that runs now, when it moves files.
    progress: Option<CopyProgress>,
    /// This device runs the library's services (Telegram, WhatsApp, the
    /// webhook, the autonomous agent, the AI actions and tasks).
    runs_services: bool,
    /// The state of the link goes in a system notification (Android), so
    /// the window shows no notice of its own.
    status_in_notification: bool,
}

fn view(app: &AppHandle) -> ConnectionView {
    let settings = settings(app);
    ConnectionView {
        mode: settings.mode,
        host_address: settings.host_address,
        can_host: cfg!(not(any(target_os = "android", target_os = "ios"))),
        server: server_view(app, settings.port),
        link: crate::host_client::link_view(app),
        copy_needs_folder: cfg!(target_os = "android"),
        copy_folder: settings.copy_folder.as_ref().map(|folder| folder.name.clone()),
        copy_ready: crate::host_mirror::copy_ready(app),
        copy: crate::host_mirror::last_status(app),
        progress: crate::host_mirror::is_transferring(app)
            .then(|| crate::host_mirror::progress(app))
            .flatten()
            .map(|(done, total)| CopyProgress { done, total }),
        runs_services: crate::device_role::runs_services(app),
        status_in_notification: cfg!(target_os = "android"),
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn server_view(app: &AppHandle, _port: u16) -> crate::host_client::ServerView {
    crate::host_server::view(app)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn server_view(_app: &AppHandle, port: u16) -> crate::host_client::ServerView {
    crate::host_client::ServerView { listening: false, port, error: None }
}

pub(crate) fn connection_settings(app: &AppHandle) -> ConnectionView {
    view(app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveConnectionPayload {
    mode: RunMode,
    #[serde(default)]
    host_address: String,
    /// «Guardar» of the address: trusts the certificate the host shows next,
    /// even for the same address (the host was reinstalled).
    #[serde(default)]
    trust_new_certificate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedConnection {
    connection: ConnectionView,
    /// The interface reloads: it opens another library.
    reload: bool,
}

/// The settings to store and whether the interface reloads.
fn next_settings(previous: &ConnectionSettings, payload: SaveConnectionPayload) -> Result<(ConnectionSettings, bool), BackendError> {
    let host_address = match payload.mode {
        RunMode::Client => parse_host_address(&payload.host_address)?.display(),
        // The address stays for when the person goes back to Client.
        RunMode::Host => parse_host_address(&payload.host_address)
            .map(|address| address.display())
            .unwrap_or_else(|_| previous.host_address.clone()),
    };
    let next = normalize_connection(ConnectionSettings {
        mode: payload.mode,
        device_id: previous.device_id.clone(),
        // Another host brings another certificate, trusted on first use.
        host_certificate: previous
            .host_certificate
            .clone()
            .filter(|_| host_address == previous.host_address && !payload.trust_new_certificate),
        host_address,
        port: previous.port,
        copy_folder: previous.copy_folder.clone(),
    });
    // On Android the copy lives in a folder the person chooses.
    if cfg!(target_os = "android") && next.mode == RunMode::Client && next.copy_folder.is_none() {
        return Err(BackendError::invalid_input("Elegí la carpeta donde guardar la copia antes de guardar el host."));
    }
    let reload = next.mode != previous.mode || (next.mode == RunMode::Client && next.host_address != previous.host_address);
    Ok((next, reload))
}

/// Saves the mode of this device and applies it: the server starts or
/// stops, and a client syncs with its host.
pub(crate) fn save_connection_settings(app: &AppHandle, payload: SaveConnectionPayload) -> Result<SavedConnection, BackendError> {
    let previous = settings(app);
    let (next, reload) = next_settings(&previous, payload)?;
    // Back to Host: this device uses the library it had before the copy.
    if previous.mode == RunMode::Client && next.mode == RunMode::Host {
        crate::host_mirror::close_copy(app)?;
    }
    store(app, &next)?;
    start(app);
    Ok(SavedConnection { connection: view(app), reload })
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestHostPayload {
    /// The saved host when absent.
    #[serde(default)]
    host_address: Option<String>,
}

pub(crate) async fn test_host_connection(app: AppHandle, payload: TestHostPayload) -> Result<crate::host_client::HostProbe, BackendError> {
    let saved = settings(&app);
    let text = payload.host_address.unwrap_or_else(|| saved.host_address.clone());
    let address = parse_host_address(&text)?;
    // The pin applies only to the saved host.
    let pin = saved.host_certificate.clone().filter(|_| address.display() == saved.host_address);
    Ok(crate::host_client::probe(&app, &address, pin).await)
}

/// Where the window of a client opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ClientOpening {
    /// On the copy, as a library of this device.
    Copy,
    /// There is no copy yet: the window signs in with the host's Owner and
    /// waits for the first sync.
    FirstSync,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenedClient {
    opening: ClientOpening,
    /// Why the copy did not open, when it is ready but failed.
    message: Option<String>,
    connection: ConnectionView,
}

/// Opens the window of a client: on its copy, at once, whether the host
/// answers or not; the copy syncs in the background. The first time there
/// is no copy yet. Right after the first sync, the Owner's session with
/// the host also opens the copy.
pub(crate) async fn client_open(app: AppHandle) -> Result<OpenedClient, BackendError> {
    let (opening, message) = if crate::host_mirror::copy_ready(&app) {
        match crate::host_mirror::open_copy(&app) {
            Ok(library_id) => {
                if let Some((username, password, remember)) = crate::host_client::session_credentials(&app) {
                    let app = app.clone();
                    let unlocked = crate::host::async_runtime::spawn_blocking(move || {
                        crate::app_auth::unlock_copy(&app, &library_id, &username, &password, remember)
                    })
                    .await;
                    if !matches!(unlocked, Ok(Ok(()))) {
                        log::error!("[notia:connection] the copy did not open with the host's session");
                    }
                }
                (ClientOpening::Copy, None)
            }
            Err(error) => (ClientOpening::FirstSync, Some(error.message)),
        }
    } else {
        (ClientOpening::FirstSync, None)
    };
    crate::client_status::refresh(&app);
    Ok(OpenedClient { opening, message, connection: view(&app) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AppPaths, HostPorts};

    fn app(directory: &std::path::Path) -> AppHandle {
        let app = crate::host::AppContext::new(AppPaths::new(Some(directory.to_path_buf()), None), HostPorts::default());
        app.manage(ConnectionState::default());
        app
    }

    #[test]
    fn a_new_host_drops_the_pin_and_mode_changes_reload() {
        let pinned = ConnectionSettings {
            mode: RunMode::Client,
            device_id: "este".into(),
            host_address: "casa:6000".into(),
            host_certificate: Some("ab".repeat(32)),
            ..ConnectionSettings::default()
        };
        let payload = |mode, address: &str| SaveConnectionPayload { mode, host_address: address.into(), trust_new_certificate: false };

        let (same, reload) = next_settings(&pinned, payload(RunMode::Client, "Casa:6000")).expect("same");
        assert!(same.host_certificate.is_some() && !reload);
        assert_eq!(same.device_id, "este");
        let (other, reload) = next_settings(&pinned, payload(RunMode::Client, "otra")).expect("other");
        assert_eq!((other.host_address.as_str(), other.host_certificate.is_none(), reload), ("otra:52480", true, true));
        // Back to Host keeps the address for later.
        let (host, reload) = next_settings(&pinned, payload(RunMode::Host, "")).expect("host");
        assert_eq!((host.mode, host.host_address.as_str(), reload), (RunMode::Host, "casa:6000", true));
        assert!(next_settings(&pinned, payload(RunMode::Client, "no válido")).is_err());
        let trust = SaveConnectionPayload { trust_new_certificate: true, ..payload(RunMode::Client, "casa:6000") };
        assert!(next_settings(&pinned, trust).expect("trust").0.host_certificate.is_none());
    }

    #[test]
    fn settings_are_stored_per_device() {
        let directory = std::env::temp_dir().join(format!("notia-connection-{}", uuid::Uuid::new_v4()));
        let app = app(&directory);
        let fresh = settings(&app);
        assert_eq!(ConnectionSettings { device_id: String::new(), ..fresh.clone() }, ConnectionSettings::default());
        // The id of the device is made once and kept.
        assert!(!fresh.device_id.is_empty());
        assert_eq!(settings(&self::app(&directory)).device_id, fresh.device_id);
        assert!(!is_client(&app));

        let pinned = ConnectionSettings {
            mode: RunMode::Client,
            device_id: fresh.device_id.clone(),
            host_address: "casa:6000".into(),
            host_certificate: Some("ab".repeat(32)),
            ..ConnectionSettings::default()
        };
        store(&app, &pinned).expect("write");
        assert!(is_client(&app));
        let (address, pin) = client_host(&app).expect("client");
        assert_eq!((address.display().as_str(), pin.is_some()), ("casa:6000", true));

        // Read back from the file by a fresh state.
        let reread = self::app(&directory);
        assert_eq!(settings(&reread), pinned);

        // A client file without host falls back to Host.
        std::fs::write(directory.join(FILE_NAME), r#"{"mode":"client","hostAddress":"","deviceId":"x"}"#).expect("write");
        assert_eq!(settings(&self::app(&directory)).mode, RunMode::Host);
        let _ = std::fs::remove_dir_all(directory);
    }
}
