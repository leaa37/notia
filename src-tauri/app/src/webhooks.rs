//! Webhook endpoint of the host PC (see `backend::webhooks`): another
//! program sends `POST /webhook` with `{ action, prompt }` and the token of a
//! registered action, and the prompt becomes a turn of the Owner's Telegram
//! chat. This adapter listens on the port of the selected library's
//! `webhook` section, checks each call with the core, queues it in the
//! Telegram worker and keeps the recent calls for Settings → Webhook.
//!
//! It runs where the Telegram bot runs: the desktop host, never Android nor
//! a client of a host. Plain HTTP: the traffic stays on this PC unless the
//! Owner lets the network call it.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::backend::webhooks::{self as core, ActionInput, CallHistory, CallRecord, CallStatus, WebhookConfig, CALLS_PER_MINUTE, TOKEN_BYTES, WEBHOOK_PATH};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Manager};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod listener;

/// Android and clients never listen (see `is_supported`).
#[cfg(any(target_os = "android", target_os = "ios"))]
mod listener {
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;

    use crate::host::AppHandle;

    #[derive(Debug, Default)]
    pub(super) struct Rates;

    pub(super) fn accept(_listener: TcpListener, _app: &AppHandle, _library_id: &str, _stop: &AtomicBool) {}

    pub(super) fn network_urls(_port: u16) -> Vec<String> {
        Vec::new()
    }
}

const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(3);
/// The section is read again after this, besides on every configuration
/// Notia writes (a change made outside Notia).
const CONFIG_TTL: Duration = Duration::from_secs(60);
/// A port that could not be opened is tried again after this.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Where the endpoint listens: the library and its port, and whether the
/// network may call it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    library_id: String,
    port: u16,
    network: bool,
}

struct Listener {
    target: Target,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Stops accepting and waits for the port to be free.
    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(crate) struct WebhookState {
    listener: Mutex<Option<Listener>>,
    /// Why `Target` could not listen, and when it was tried.
    failure: Mutex<Option<(Target, String, Instant)>>,
    /// The section's target at a configuration generation, and when it was read.
    decision: Mutex<Option<(u64, String, Instant, Option<Target>)>>,
    calls: Mutex<CallHistory>,
    #[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
    rates: Mutex<listener::Rates>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as i64).unwrap_or_default()
}

/// Whether this device listens: the computer that runs the library's
/// services (`device_role`), never Android.
fn is_supported(app: &AppHandle) -> bool {
    crate::device_role::runs_services(app)
}

fn unsupported_reason() -> &'static str {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        "El webhook escucha en la computadora que atiende Telegram, no en Android. Configuralo desde esa computadora."
    } else {
        "El webhook escucha en la computadora que atiende Telegram y los demás servicios de la biblioteca: ahora es otra, con más procesamiento."
    }
}

fn read_config(app: &AppHandle, library_id: &str) -> Result<WebhookConfig, BackendError> {
    Ok(WebhookConfig::from_library_config(crate::library_config::read_library_config(app, library_id)?.as_ref()))
}

fn save_config(app: &AppHandle, library_id: &str, section: &WebhookConfig) -> Result<(), BackendError> {
    crate::library_config::update_library_config(app, library_id, |config| core::with_webhook(&config, section))?;
    // The supervisor follows the change now, not on its next look.
    lock(&app.state::<WebhookState>().decision).take();
    reconcile(app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Plugin that keeps the endpoint in line with the selected library and its
/// webhook section.
pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("webhook-endpoint")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-webhook-supervisor".into()).spawn(move || loop {
                std::thread::sleep(SUPERVISOR_INTERVAL);
                reconcile(&app);
            });
            Ok(())
        })
        .build()
}

/// Where the endpoint should listen now.
fn desired_target(app: &AppHandle) -> Option<Target> {
    if !is_supported(app) {
        return None;
    }
    let library = crate::library_catalog::selected_library_id(app)?;
    let generation = crate::library_config::config_generation();
    let state = app.state::<WebhookState>();
    let cached = lock(&state.decision)
        .as_ref()
        .filter(|(seen, id, at, _)| *seen == generation && *id == library && at.elapsed() < CONFIG_TTL)
        .map(|(.., target)| target.clone());
    if let Some(target) = cached {
        return target;
    }
    let bound = app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library).is_ok();
    let target = bound
        .then(|| read_config(app, &library).ok())
        .flatten()
        .filter(|config| config.enabled)
        .map(|config| Target { library_id: library.clone(), port: config.port, network: config.network });
    *lock(&state.decision) = Some((generation, library, Instant::now(), target.clone()));
    target
}

fn reconcile(app: &AppHandle) {
    let state = app.state::<WebhookState>();
    let mut current = lock(&state.listener);
    let desired = desired_target(app);
    if current.as_ref().map(|listener| &listener.target) == desired.as_ref() {
        return;
    }
    if let Some(previous) = current.take() {
        previous.stop();
    }
    let Some(target) = desired else {
        lock(&state.failure).take();
        return;
    };
    let mut failure = lock(&state.failure);
    if failure.as_ref().is_some_and(|(failed, _, at)| *failed == target && at.elapsed() < RETRY_AFTER) {
        return;
    }
    let host = if target.network { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
    match TcpListener::bind(SocketAddr::from((host, target.port))) {
        Ok(listener) => {
            failure.take();
            let stop = Arc::new(AtomicBool::new(false));
            let thread = std::thread::Builder::new().name("notia-webhook".into()).spawn({
                let app = app.clone();
                let library_id = target.library_id.clone();
                let stop = Arc::clone(&stop);
                move || listener::accept(listener, &app, &library_id, &stop)
            });
            match thread {
                Ok(thread) => *current = Some(Listener { target, stop, thread: Some(thread) }),
                Err(_) => *failure = Some((target, "No se pudo iniciar el webhook.".into(), Instant::now())),
            }
        }
        Err(error) => {
            let message = if error.kind() == std::io::ErrorKind::AddrInUse {
                format!("El puerto {} ya lo usa otro programa: elegí otro.", target.port)
            } else {
                format!("No se pudo abrir el puerto {}.", target.port)
            };
            log::error!("[notia:webhook] no se pudo escuchar en el puerto {}: {:?}", target.port, error.kind());
            *failure = Some((target, message, Instant::now()));
        }
    }
}

/// The port the endpoint uses now, so no other service of this device takes it.
pub(crate) fn listening_port(app: &AppHandle) -> Option<u16> {
    let state = app.try_state::<WebhookState>()?;
    let port = lock(&state.listener).as_ref().map(|listener| listener.target.port);
    port
}

// ---------------------------------------------------------------------------
// Calls (the HTTP part is in `listener`)
// ---------------------------------------------------------------------------

pub(crate) fn mark_running(app: &AppHandle, call_id: &str) {
    lock(&app.state::<WebhookState>().calls).mark_running(call_id);
}

pub(crate) fn finish_call(app: &AppHandle, call_id: &str, outcome: Result<String, String>) {
    let state = app.state::<WebhookState>();
    let mut calls = lock(&state.calls);
    if let Some(call) = calls.finish(call_id, outcome) {
        if call.status == CallStatus::Failed {
            log::error!("[notia:webhook] una llamada de «{}» no terminó", call.action);
        }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebhookPayload {
    library_id: String,
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    network: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionView {
    id: String,
    name: String,
    instructions: String,
    enabled: bool,
    created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_call_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebhookView {
    /// This device listens (the desktop host).
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<&'static str>,
    enabled: bool,
    port: u16,
    network: bool,
    /// `off`, `starting`, `listening` or `error`.
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// Addresses a program calls while it listens.
    urls: Vec<String>,
    /// Where the example call of the settings points.
    example_url: String,
    calls_per_minute: usize,
    /// The Telegram bot of the library runs here: calls can run.
    telegram: bool,
    actions: Vec<ActionView>,
    calls: Vec<CallRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebhookSaved {
    view: WebhookView,
    /// The action's new token, shown only this once.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_id: Option<String>,
}

fn view(app: &AppHandle, library_id: &str) -> Result<WebhookView, BackendError> {
    let config = read_config(app, library_id)?;
    let available = is_supported(app);
    let state = app.state::<WebhookState>();
    let listening = lock(&state.listener).as_ref().map(|listener| listener.target.clone()).filter(|target| target.library_id == library_id);
    let failed = lock(&state.failure).as_ref().filter(|(target, ..)| target.library_id == library_id).map(|(_, message, _)| message.clone());
    let (state_id, message) = match (&listening, failed) {
        (Some(_), _) => ("listening", None),
        (None, Some(message)) if config.enabled => ("error", Some(message)),
        (None, _) if config.enabled && available => ("starting", Some("Abriendo el puerto…".to_string())),
        _ => ("off", None),
    };
    let urls = match &listening {
        Some(target) => {
            let mut urls = vec![format!("http://127.0.0.1:{}{WEBHOOK_PATH}", target.port)];
            if target.network {
                urls.extend(listener::network_urls(target.port));
            }
            urls
        }
        None => Vec::new(),
    };
    let calls = lock(&state.calls);
    let actions = config
        .actions
        .iter()
        .map(|action| ActionView {
            id: action.id.clone(),
            name: action.name.clone(),
            instructions: action.instructions.clone(),
            enabled: action.enabled,
            created_at_ms: action.created_at_ms,
            last_call_ms: calls.last_call_ms(library_id, &action.name),
        })
        .collect();
    Ok(WebhookView {
        available,
        unavailable_reason: (!available).then(|| unsupported_reason()),
        enabled: config.enabled,
        port: config.port,
        network: config.network,
        state: state_id,
        message,
        example_url: urls.first().cloned().unwrap_or_else(|| format!("http://127.0.0.1:{}{WEBHOOK_PATH}", config.port)),
        urls,
        calls_per_minute: CALLS_PER_MINUTE,
        telegram: crate::telegram_worker::bot_runs_for(app, library_id),
        actions,
        calls: calls.of_library(library_id),
    })
}

/// Ports other services of this device use.
fn reserved_ports(app: &AppHandle) -> Vec<(u16, String)> {
    let mut reserved = vec![(crate::connection::settings(app).port, "el servidor del host".to_string())];
    if let Some(port) = crate::device_preferences::section(app, "taskManagerPublication")["port"].as_u64().and_then(|port| u16::try_from(port).ok()) {
        reserved.push((port, "el tablero publicado".to_string()));
    }
    reserved.extend(crate::public_spaces::stored_spaces(app).into_iter().map(|space| (space.port, format!("el espacio público «{}»", space.name))));
    reserved
}

fn new_token() -> (String, String) {
    let mut bytes = [0_u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let hash = core::token_hash(&token);
    (token, hash)
}

fn action_id(payload: &WebhookPayload) -> Result<&str, BackendError> {
    payload.action_id.as_deref().map(str::trim).filter(|id| !id.is_empty()).ok_or_else(|| BackendError::invalid_input("Falta la acción de webhook."))
}

async fn blocking<T: Send + 'static>(task: impl FnOnce() -> Result<T, BackendError> + Send + 'static) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo completar la operación del webhook.", true))?
}

/// The endpoint, its actions and the recent calls.
pub(crate) async fn webhook_view(app: AppHandle, payload: WebhookPayload) -> Result<WebhookView, BackendError> {
    blocking(move || view(&app, &payload.library_id)).await
}

/// Turns the endpoint on or off and sets its port and who may call it.
pub(crate) async fn webhook_save_server(app: AppHandle, payload: WebhookPayload) -> Result<WebhookView, BackendError> {
    blocking(move || {
        if payload.enabled && !is_supported(&app) {
            return Err(BackendError::new(BackendErrorCode::Unsupported, unsupported_reason(), false));
        }
        let mut config = read_config(&app, &payload.library_id)?;
        let port = payload.port.unwrap_or(config.port);
        let reserved = reserved_ports(&app);
        let reserved: Vec<(u16, &str)> = reserved.iter().map(|(port, service)| (*port, service.as_str())).collect();
        config.set_server(payload.enabled, port, payload.network, &reserved)?;
        save_config(&app, &payload.library_id, &config)?;
        view(&app, &payload.library_id)
    })
    .await
}

/// Registers an action (with its first token) or changes one.
pub(crate) async fn webhook_save_action(app: AppHandle, payload: WebhookPayload) -> Result<WebhookSaved, BackendError> {
    blocking(move || {
        let mut config = read_config(&app, &payload.library_id)?;
        let input = ActionInput { name: &payload.name, instructions: &payload.instructions, enabled: payload.enabled };
        let (token, id) = match payload.action_id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => (None, config.update_action(id, input)?.id.clone()),
            None => {
                let (token, hash) = new_token();
                (Some(token), config.add_action(uuid::Uuid::new_v4().to_string(), input, hash, now_ms())?.id.clone())
            }
        };
        save_config(&app, &payload.library_id, &config)?;
        Ok(WebhookSaved { view: view(&app, &payload.library_id)?, token, action_id: Some(id) })
    })
    .await
}

/// A new token for an action; the previous one stops working.
pub(crate) async fn webhook_new_token(app: AppHandle, payload: WebhookPayload) -> Result<WebhookSaved, BackendError> {
    blocking(move || {
        let id = action_id(&payload)?.to_string();
        let mut config = read_config(&app, &payload.library_id)?;
        let (token, hash) = new_token();
        config.replace_token(&id, hash)?;
        save_config(&app, &payload.library_id, &config)?;
        Ok(WebhookSaved { view: view(&app, &payload.library_id)?, token: Some(token), action_id: Some(id) })
    })
    .await
}

pub(crate) async fn webhook_delete_action(app: AppHandle, payload: WebhookPayload) -> Result<WebhookView, BackendError> {
    blocking(move || {
        let id = action_id(&payload)?.to_string();
        let mut config = read_config(&app, &payload.library_id)?;
        config.remove_action(&id)?;
        save_config(&app, &payload.library_id, &config)?;
        view(&app, &payload.library_id)
    })
    .await
}
