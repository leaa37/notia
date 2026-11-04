//! The Owner's WhatsApp on the host PC: a supervisor keeps one link per
//! selected library whose configuration has WhatsApp on. The link installs
//! Baileys with the PC's npm the first time (`node_packages`), runs
//! `whatsapp_agent.mjs` and keeps what it reports (the QR, the connection,
//! the chats and the recent messages) in a store of the app data. On it run
//! the assistant's tools, the WhatsApp settings and the auto replies: a
//! message that arrives in a chat with auto reply waits a minute from the
//! last one (`reply_wait`), and then the model reads
//! everything that arrived and writes one answer with the chat's prompt.
//!
//! What the tools may read or send, the store, the prompts and the texts
//! live in `backend::whatsapp`. Android has no Node.js, and a client device
//! uses its host's link; neither links WhatsApp. Logs never carry numbers,
//! names or messages.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead as _, BufReader, Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use chrono::TimeZone as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::whatsapp::{
    self as core, ChatAccess, ChatSettings, ChatUpdate, ChatView, ContactUpdate, WhatsAppCommand, WhatsAppConfig, WhatsAppStore, WireMessage,
};
use crate::backend::{BackendError, BackendErrorCode, BackendRequestContext};
use crate::host::{AppHandle, Manager};
use crate::node_packages::{hide_console, NodePackage};

const AGENT_PACKAGE: NodePackage = NodePackage {
    name: "notia-whatsapp-agent",
    script_name: "whatsapp_agent.mjs",
    script: include_str!("whatsapp_agent.mjs"),
    packages: &[("baileys", "7.0.0-rc14"), ("qrcode", "1.5.4")],
    min_node_major: 20,
    needed_by: "WhatsApp",
};
const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(3);
/// The configuration is read again at least this often (a change made
/// outside Notia).
const CONFIG_TTL: Duration = Duration::from_secs(60);
/// The store is written at most this often while messages arrive.
const STORE_SAVE_INTERVAL: Duration = Duration::from_secs(15);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
const FILE_ANSWER_TIMEOUT: Duration = Duration::from_secs(120);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(15);
/// A process that ended by itself starts again after this long; one that
/// could not be installed waits longer before npm runs again.
const RESTART_DELAY: Duration = Duration::from_secs(10);
const INSTALL_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const AUTO_REPLY_TIMEOUT: Duration = Duration::from_secs(180);
/// Chats the settings list at once; the search finds the rest.
const SETTINGS_CHAT_LIMIT: usize = 200;
/// An image reaches the model at most this many pixels on its longest side.
const IMAGE_MAX_SIDE: u32 = 1600;
const IMAGE_QUALITY: u8 = 80;

#[derive(Default)]
pub(crate) struct WhatsAppState {
    link: Mutex<Option<Arc<Link>>>,
    /// What the settings say after the link stopped by itself (WhatsApp
    /// unlinked Notia), by library.
    notice: Mutex<Option<(String, String)>>,
    /// What the supervisor read last: (configuration generation, library,
    /// when, whether WhatsApp is on).
    decision: Mutex<Option<(u64, String, Instant, bool)>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum LinkState {
    /// Installing Baileys or starting the process.
    #[default]
    Starting,
    /// Waiting for the QR to be scanned.
    Qr,
    Open,
    /// The connection dropped; the process connects again by itself.
    Closed,
    /// It could not start (no Node.js, npm failed).
    Error,
}

#[derive(Debug, Clone, Default)]
struct LinkStatus {
    state: LinkState,
    qr_svg: Option<String>,
    account_id: Option<String>,
    account_name: Option<String>,
    message: Option<String>,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The WhatsApp of one library on this PC.
struct Link {
    library_id: String,
    folder: PathBuf,
    process: Mutex<Option<Process>>,
    /// Whether a start (install and spawn) is under way.
    starting: AtomicBool,
    /// When the process may start again after it ended or failed.
    restart_at: Mutex<Option<Instant>>,
    status: Mutex<LinkStatus>,
    config: Mutex<WhatsAppConfig>,
    /// The configuration generation `config` was read at, and when.
    config_read: Mutex<(u64, Instant)>,
    store: Mutex<WhatsAppStore>,
    dirty: AtomicBool,
    saved_at: Mutex<Instant>,
    next_id: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
    /// Chats whose auto reply waits, and until when.
    replies: Mutex<HashMap<String, Instant>>,
    /// Chats whose auto reply is being written.
    replying: Mutex<HashSet<String>>,
    stopped: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether this device links WhatsApp: the computer that runs the
/// library's services (`device_role`), never Android.
fn is_supported(app: &AppHandle) -> bool {
    crate::device_role::runs_services(app)
}

fn unsupported_reason() -> &'static str {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        "WhatsApp se vincula en la computadora que atiende Telegram (necesita Node.js), no en Android."
    } else {
        "WhatsApp se vincula en la computadora que atiende Telegram y los demás servicios de la biblioteca: ahora es otra, con más procesamiento."
    }
}

fn not_connected() -> BackendError {
    BackendError::new(
        BackendErrorCode::Conflict,
        "WhatsApp no está conectado ahora: la persona lo conecta en Configuración → WhatsApp.",
        true,
    )
}

/// The moment `ms` in the host's local time, as the model reads it.
fn local_time(ms: u64) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single())
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn data_folder(app: &AppHandle, library_id: &str) -> Option<PathBuf> {
    let key = library_id.chars().filter(|character| character.is_ascii_alphanumeric() || *character == '-').take(64).collect::<String>();
    app.path().app_data_dir().ok().map(|directory| directory.join("whatsapp").join(key))
}

fn package_folder(app: &AppHandle) -> Result<PathBuf, BackendError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("whatsapp-agent"))
        .map_err(|_| BackendError::new(BackendErrorCode::Storage, "No se pudo resolver la carpeta de WhatsApp.", true))
}

fn read_config(app: &AppHandle, library_id: &str) -> Result<WhatsAppConfig, BackendError> {
    Ok(WhatsAppConfig::from_library_config(crate::library_config::read_library_config(app, library_id)?.as_ref()))
}

fn save_config(app: &AppHandle, library_id: &str, section: &WhatsAppConfig) -> Result<(), BackendError> {
    crate::library_config::update_library_config(app, library_id, |config| core::with_whatsapp(&config, section)).map(|_| ())
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Plugin that keeps the link in line with the selected library and its
/// WhatsApp section, saves the store and sends the auto replies that are due.
pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("whatsapp-link")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-whatsapp-supervisor".into()).spawn(move || loop {
                std::thread::sleep(SUPERVISOR_INTERVAL);
                reconcile(&app);
            });
            Ok(())
        })
        .build()
}

/// The library whose WhatsApp should be linked now.
fn desired_library(app: &AppHandle) -> Option<String> {
    if !is_supported(app) {
        return None;
    }
    let library = crate::library_catalog::selected_library_id(app)?;
    let generation = crate::library_config::config_generation();
    let state = app.state::<WhatsAppState>();
    let cached = lock(&state.decision)
        .as_ref()
        .filter(|(seen, id, at, _)| *seen == generation && *id == library && at.elapsed() < CONFIG_TTL)
        .map(|(.., enabled)| *enabled);
    let enabled = match cached {
        Some(enabled) => enabled,
        None => {
            let bound = app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library).is_ok();
            let enabled = bound && read_config(app, &library).is_ok_and(|config| config.enabled);
            *lock(&state.decision) = Some((generation, library.clone(), Instant::now(), enabled));
            enabled
        }
    };
    enabled.then_some(library)
}

fn reconcile(app: &AppHandle) {
    let state = app.state::<WhatsAppState>();
    let link = {
        // Decided under the lock: a disconnect holds it while it turns
        // WhatsApp off, so its link is never opened again meanwhile.
        let mut current = lock(&state.link);
        let desired = desired_library(app);
        if current.as_ref().map(|link| link.library_id.as_str()) != desired.as_deref() {
            if let Some(previous) = current.take() {
                previous.stop();
            }
            if let Some(library_id) = desired.as_deref() {
                *current = Link::open(app, library_id).map(Arc::new);
            }
        }
        current.clone()
    };
    let Some(link) = link else { return };
    link.refresh_config(app);
    link.ensure_running(app);
    link.save_store(false);
    link.send_due_replies(app);
}

/// The link of `library_id` when it runs on this device.
fn link_for(app: &AppHandle, library_id: &str) -> Option<Arc<Link>> {
    lock(&app.state::<WhatsAppState>().link).as_ref().filter(|link| link.library_id == library_id).cloned()
}

/// Whether the assistant has WhatsApp now for `library_id`: linked here
/// and connected.
pub(crate) fn is_connected(app: &AppHandle, library_id: &str) -> bool {
    link_for(app, library_id).is_some_and(|link| lock(&link.status).state == LinkState::Open)
}

// ---------------------------------------------------------------------------
// The link and its process
// ---------------------------------------------------------------------------

impl Link {
    fn open(app: &AppHandle, library_id: &str) -> Option<Link> {
        let folder = data_folder(app, library_id)?;
        let store = std::fs::read_to_string(folder.join("store.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<WhatsAppStore>(&text).ok())
            .unwrap_or_default();
        Some(Link {
            library_id: library_id.to_string(),
            folder,
            process: Mutex::new(None),
            starting: AtomicBool::new(false),
            restart_at: Mutex::new(None),
            status: Mutex::new(LinkStatus { message: Some("Preparando WhatsApp…".into()), ..LinkStatus::default() }),
            config: Mutex::new(read_config(app, library_id).unwrap_or_default()),
            config_read: Mutex::new((crate::library_config::config_generation(), Instant::now())),
            store: Mutex::new(store),
            dirty: AtomicBool::new(false),
            saved_at: Mutex::new(Instant::now()),
            next_id: AtomicU64::new(0),
            waiting: Mutex::new(HashMap::new()),
            replies: Mutex::new(HashMap::new()),
            replying: Mutex::new(HashSet::new()),
            stopped: AtomicBool::new(false),
        })
    }

    fn config(&self) -> WhatsAppConfig {
        lock(&self.config).clone()
    }

    /// Reads the configuration again when Notia wrote one, or after
    /// [`CONFIG_TTL`].
    fn refresh_config(&self, app: &AppHandle) {
        let generation = crate::library_config::config_generation();
        {
            let mut read = lock(&self.config_read);
            if read.0 == generation && read.1.elapsed() < CONFIG_TTL {
                return;
            }
            *read = (generation, Instant::now());
        }
        if let Ok(config) = read_config(app, &self.library_id) {
            let mut replies = lock(&self.replies);
            replies.retain(|chat, _| config.auto_reply_enabled(chat));
            *lock(&self.config) = config;
        }
    }

    fn set_status(&self, change: impl FnOnce(&mut LinkStatus)) {
        change(&mut lock(&self.status));
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        lock(&self.process).take();
        self.save_store(true);
    }

    /// Starts the process when it is not running and may start.
    fn ensure_running(self: &Arc<Self>, app: &AppHandle) {
        if self.stopped.load(Ordering::SeqCst) || lock(&self.process).is_some() || self.starting.load(Ordering::SeqCst) {
            return;
        }
        if lock(&self.restart_at).is_some_and(|at| Instant::now() < at) {
            return;
        }
        self.starting.store(true, Ordering::SeqCst);
        let link = Arc::clone(self);
        let app = app.clone();
        let spawned = std::thread::Builder::new().name("notia-whatsapp-start".into()).spawn(move || {
            if let Err(error) = link.start(&app) {
                log::error!("[notia:whatsapp] no se pudo iniciar WhatsApp");
                *lock(&link.restart_at) = Some(Instant::now() + INSTALL_RETRY_DELAY);
                link.set_status(|status| {
                    *status = LinkStatus { state: LinkState::Error, message: Some(error.message), ..LinkStatus::default() };
                });
            }
            link.starting.store(false, Ordering::SeqCst);
        });
        if spawned.is_err() {
            self.starting.store(false, Ordering::SeqCst);
        }
    }

    fn start(self: &Arc<Self>, app: &AppHandle) -> Result<(), BackendError> {
        self.set_status(|status| {
            status.state = LinkState::Starting;
            status.message = Some("Preparando WhatsApp (la primera vez se instala Baileys con npm)…".into());
        });
        let package = package_folder(app)?;
        AGENT_PACKAGE.install_into(&package)?;
        std::fs::create_dir_all(&self.folder).map_err(|_| BackendError::new(BackendErrorCode::Storage, "No se pudo preparar la carpeta de WhatsApp.", true))?;
        let mut command = Command::new("node");
        command
            .arg(package.join(AGENT_PACKAGE.script_name))
            .arg(&self.folder)
            .current_dir(&package)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        hide_console(&mut command);
        let mut child = command.spawn().map_err(|_| AGENT_PACKAGE.node_missing())?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(BackendError::new(BackendErrorCode::Internal, "No se pudo conectar con el proceso de WhatsApp.", true));
        };
        if self.stopped.load(Ordering::SeqCst) {
            let _ = child.kill();
            return Ok(());
        }
        *lock(&self.process) = Some(Process { child, stdin });
        self.set_status(|status| status.message = Some("Conectando con WhatsApp…".into()));
        let link = Arc::clone(self);
        let app = app.clone();
        let spawned = std::thread::Builder::new().name("notia-whatsapp-read".into()).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Value>(&line) {
                    Ok(value) => link.receive(&app, value),
                    Err(_) => log::error!("[notia:whatsapp] línea ilegible del proceso de WhatsApp"),
                }
            }
            link.process_ended();
        });
        if spawned.is_err() {
            lock(&self.process).take();
            return Err(BackendError::new(BackendErrorCode::Internal, "No se pudo leer el proceso de WhatsApp.", true));
        }
        Ok(())
    }

    /// The process ended: its pending answers fail and it starts again
    /// later, unless the link stopped.
    fn process_ended(&self) {
        lock(&self.process).take();
        lock(&self.waiting).clear();
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        *lock(&self.restart_at) = Some(Instant::now() + RESTART_DELAY);
        self.set_status(|status| {
            if status.state != LinkState::Error {
                status.state = LinkState::Closed;
                status.message = Some("Se cortó la conexión con WhatsApp; Notia vuelve a intentar en unos segundos.".into());
            }
        });
    }

    /// Sends a request and waits for its answer.
    fn request(&self, mut body: Value, timeout: Duration) -> Result<Value, BackendError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        body["id"] = json!(id);
        let (sender, answer) = mpsc::channel();
        lock(&self.waiting).insert(id, sender);
        let written = {
            let mut process = lock(&self.process);
            match process.as_mut() {
                Some(process) => process.stdin.write_all(format!("{body}\n").as_bytes()).and_then(|()| process.stdin.flush()).is_ok(),
                None => false,
            }
        };
        if !written {
            lock(&self.waiting).remove(&id);
            return Err(not_connected());
        }
        let answer = match answer.recv_timeout(timeout) {
            Ok(answer) => answer,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.waiting).remove(&id);
                return Err(BackendError::new(BackendErrorCode::Conflict, "WhatsApp no respondió a tiempo; probá de nuevo.", true));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(not_connected()),
        };
        if answer.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(answer)
        } else {
            let message = answer.get("error").and_then(Value::as_str).unwrap_or("WhatsApp rechazó el pedido.");
            Err(BackendError::new(BackendErrorCode::Conflict, message.to_string(), true))
        }
    }

    fn receive(&self, app: &AppHandle, value: Value) {
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            if let Some(sender) = lock(&self.waiting).remove(&id) {
                let _ = sender.send(value);
            }
            return;
        }
        let parse = |key: &str| value.get(key).cloned().unwrap_or(Value::Null);
        match value.get("event").and_then(Value::as_str) {
            Some("qr") => self.set_status(|status| {
                status.state = LinkState::Qr;
                status.qr_svg = value.get("svg").and_then(Value::as_str).map(str::to_string);
                status.message = None;
            }),
            Some("state") => self.receive_state(app, &value),
            Some("chats") => {
                if let Ok(chats) = serde_json::from_value::<Vec<ChatUpdate>>(parse("chats")) {
                    lock(&self.store).upsert_chats(chats);
                    self.dirty.store(true, Ordering::SeqCst);
                }
            }
            Some("contacts") => {
                if let Ok(contacts) = serde_json::from_value::<Vec<ContactUpdate>>(parse("contacts")) {
                    lock(&self.store).upsert_contacts(contacts);
                    self.dirty.store(true, Ordering::SeqCst);
                }
            }
            Some("chatsDeleted") => {
                if let Ok(ids) = serde_json::from_value::<Vec<String>>(parse("ids")) {
                    lock(&self.store).remove_chats(&ids);
                    self.dirty.store(true, Ordering::SeqCst);
                }
            }
            Some("messages") => {
                if let Ok(messages) = serde_json::from_value::<Vec<WireMessage>>(parse("messages")) {
                    let live = value.get("live").and_then(Value::as_bool).unwrap_or(false);
                    self.receive_messages(messages, live);
                }
            }
            _ => {}
        }
    }

    fn receive_state(&self, app: &AppHandle, value: &Value) {
        match value.get("state").and_then(Value::as_str) {
            Some("connecting") => self.set_status(|status| {
                if status.state != LinkState::Qr {
                    status.state = LinkState::Starting;
                    status.message = Some("Conectando con WhatsApp…".into());
                }
            }),
            Some("open") => {
                let account = value.get("account");
                let text = |key: &str| account.and_then(|account| account.get(key)).and_then(Value::as_str).map(str::to_string);
                self.set_status(|status| {
                    *status = LinkStatus { state: LinkState::Open, account_id: text("id"), account_name: text("name"), ..LinkStatus::default() };
                });
                *lock(&self.restart_at) = None;
                log::info!("[notia:whatsapp] conectado");
            }
            Some("closed") => self.set_status(|status| {
                status.state = LinkState::Closed;
                status.message = Some("Se cortó la conexión con WhatsApp; reconectando…".into());
            }),
            Some("loggedOut") => self.unlinked(app, "WhatsApp desvinculó a Notia (desde el teléfono o por inactividad). Conectalo de nuevo para seguir."),
            Some("replaced") => self.unlinked(app, "Otra sesión abrió este mismo vínculo de WhatsApp y esta se detuvo. Conectalo de nuevo si querés usarlo acá."),
            _ => {}
        }
    }

    /// WhatsApp ended the link: it stays off until the person connects it
    /// again, and the settings say why.
    fn unlinked(&self, app: &AppHandle, reason: &str) {
        log::error!("[notia:whatsapp] WhatsApp terminó el vínculo");
        self.stopped.store(true, Ordering::SeqCst);
        self.set_status(|status| *status = LinkStatus { state: LinkState::Error, message: Some(reason.to_string()), ..LinkStatus::default() });
        *lock(&app.state::<WhatsAppState>().notice) = Some((self.library_id.clone(), reason.to_string()));
        // The session is gone, and so are its chats.
        *lock(&self.store) = WhatsAppStore::default();
        self.dirty.store(false, Ordering::SeqCst);
        let _ = std::fs::remove_file(self.folder.join("store.json"));
        let app = app.clone();
        let library_id = self.library_id.clone();
        let _ = std::thread::Builder::new().name("notia-whatsapp-unlinked".into()).spawn(move || {
            if let Ok(mut config) = read_config(&app, &library_id) {
                config.enabled = false;
                if save_config(&app, &library_id, &config).is_err() {
                    log::error!("[notia:whatsapp] no se pudo apagar WhatsApp en la configuración");
                }
            }
        });
    }

    /// Stores the messages; a new one of someone else in a chat with auto
    /// reply starts its wait again, and one the Owner wrote from the phone
    /// drops it (the Owner answered).
    fn receive_messages(&self, messages: Vec<WireMessage>, live: bool) {
        let added = lock(&self.store).add_messages(messages);
        self.dirty.store(true, Ordering::SeqCst);
        if !live {
            return;
        }
        let config = self.config();
        let mut replies = lock(&self.replies);
        for (chat, message) in added {
            if message.from_me {
                if !message.by_notia {
                    replies.remove(&chat);
                }
            } else {
                crate::ai_task_runner::whatsapp_message_arrived(&chat);
                if config.auto_reply_enabled(&chat) {
                    replies.insert(chat, Instant::now() + crate::backend::reply_wait::WHATSAPP_WAIT);
                }
            }
        }
    }

    /// Writes the store when it changed: at once with `now`, else at most
    /// every [`STORE_SAVE_INTERVAL`].
    fn save_store(&self, now: bool) {
        {
            let mut saved_at = lock(&self.saved_at);
            if !now && saved_at.elapsed() < STORE_SAVE_INTERVAL {
                return;
            }
            if !self.dirty.swap(false, Ordering::SeqCst) {
                return;
            }
            *saved_at = Instant::now();
        }
        let Ok(text) = serde_json::to_string(&*lock(&self.store)) else { return };
        if std::fs::create_dir_all(&self.folder).is_err() {
            return;
        }
        let path = self.folder.join("store.json");
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, text).is_err() || std::fs::rename(&temporary, &path).is_err() {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }

    // -----------------------------------------------------------------------
    // Auto reply
    // -----------------------------------------------------------------------

    fn send_due_replies(self: &Arc<Self>, app: &AppHandle) {
        if lock(&self.status).state != LinkState::Open {
            return;
        }
        let due = {
            let mut replies = lock(&self.replies);
            let now = Instant::now();
            let due = replies.iter().filter(|(_, at)| **at <= now).map(|(chat, _)| chat.clone()).collect::<Vec<_>>();
            for chat in &due {
                replies.remove(chat);
            }
            due
        };
        for chat in due {
            if !lock(&self.replying).insert(chat.clone()) {
                continue;
            }
            let link = Arc::clone(self);
            let app = app.clone();
            let spawned = std::thread::Builder::new().name("notia-whatsapp-reply".into()).spawn(move || {
                link.auto_reply(&app, &chat);
                lock(&link.replying).remove(&chat);
            });
            if spawned.is_err() {
                log::error!("[notia:whatsapp] no se pudo preparar una autorespuesta");
            }
        }
    }

    /// Reads what arrived in `chat` since the last own message and answers
    /// it with the chat's prompt. Nothing goes when the model chose silence,
    /// when the Owner answered meanwhile or when more messages arrived
    /// (their own wait answers them all).
    fn auto_reply(&self, app: &AppHandle, chat: &str) {
        let config = self.config();
        if !config.auto_reply_enabled(chat) {
            return;
        }
        let (answered, image_ids) = {
            let store = lock(&self.store);
            let pending = core::unanswered(store.messages(chat));
            (pending.last().map(|message| message.id.clone()), core::images_to_read(pending))
        };
        let Some(answered) = answered else { return };
        let mut images = Vec::new();
        let mut shown = Vec::new();
        for id in image_ids {
            if let Ok(image) = self.download_image(&id) {
                images.push(image);
                shown.push(id);
            }
        }
        let account_name = lock(&self.status).account_name.clone();
        let prompt = core::auto_reply_prompt(&lock(&self.store), &config, chat, account_name.as_deref(), &shown, &local_time);
        let Some((system, user)) = prompt else { return };
        let answer = match crate::backend_runtime::complete_with_images(app, &self.library_id, &system, &user, images, AUTO_REPLY_TIMEOUT) {
            Ok(answer) => answer,
            Err(_) => {
                log::error!("[notia:whatsapp] el modelo no pudo escribir una autorespuesta");
                return;
            }
        };
        let Some(text) = core::parse_auto_reply(&answer) else {
            log::info!("[notia:whatsapp] autorespuesta: el modelo eligió no responder");
            return;
        };
        let still_pending = {
            let store = lock(&self.store);
            core::unanswered(store.messages(chat)).last().map(|message| message.id.clone())
        };
        if still_pending.as_deref() != Some(answered.as_str()) || lock(&self.replies).contains_key(chat) || !self.config().auto_reply_enabled(chat) {
            return;
        }
        match self.send_text(chat, &text, false) {
            Ok(_) => log::info!("[notia:whatsapp] autorespuesta enviada"),
            Err(_) => log::error!("[notia:whatsapp] no se pudo enviar una autorespuesta"),
        }
    }

    // -----------------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------------

    /// Keeps a message Notia sent, marked as Notia's; its chat (a new
    /// number's real id) and its id.
    fn keep_sent(&self, answer: &Value) -> Option<(String, String)> {
        let mut message = serde_json::from_value::<WireMessage>(answer.get("message")?.clone()).ok()?;
        message.message.by_notia = true;
        let sent = (message.chat.clone(), message.message.id.clone());
        lock(&self.store).add_messages(vec![message]);
        self.dirty.store(true, Ordering::SeqCst);
        Some(sent)
    }

    /// Chooses `option` in the menu `menu_id` of `chat`, as tapping it on
    /// the phone does.
    fn send_option(&self, chat: &str, menu_id: &str, option: &str) -> Result<Option<(String, String)>, BackendError> {
        let answer = self.request(json!({ "op": "reply", "chat": chat, "messageId": menu_id, "option": option }), ANSWER_TIMEOUT)?;
        Ok(self.keep_sent(&answer))
    }

    /// `lookup`: the chat is a number nobody wrote to yet.
    fn send_text(&self, chat: &str, text: &str, lookup: bool) -> Result<Option<(String, String)>, BackendError> {
        let answer = self.request(json!({ "op": "send", "chat": chat, "text": text, "lookup": lookup }), ANSWER_TIMEOUT)?;
        Ok(self.keep_sent(&answer))
    }

    /// The chat a first message reached gets Lectura and Escritura, so the
    /// assistant follows the conversation it started (the person confirmed
    /// that message).
    fn allow_new_chat(&self, app: &AppHandle, chat: &str) -> Result<(), BackendError> {
        let mut config = read_config(app, &self.library_id)?;
        let current = config.chat(chat);
        let name = core::name_of(&lock(&self.store), &config, chat);
        config.set_chat(ChatSettings { name, read: true, write: true, ..current })?;
        save_config(app, &self.library_id, &config)?;
        *lock(&self.config) = config;
        Ok(())
    }

    /// The image of a message, as the model reads it (base64 JPEG).
    fn download_image(&self, message_id: &str) -> Result<String, BackendError> {
        let answer = self.request(json!({ "op": "download", "messageId": message_id }), FILE_ANSWER_TIMEOUT)?;
        let invalid = || BackendError::new(BackendErrorCode::Conflict, "La imagen de WhatsApp no se pudo leer.", true);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(answer.get("base64").and_then(Value::as_str).unwrap_or_default())
            .map_err(|_| invalid())?;
        if bytes.len() > core::MAX_IMAGE_BYTES {
            return Err(BackendError::invalid_input("La imagen es demasiado grande para mirarla."));
        }
        model_image(&bytes).ok_or_else(invalid)
    }
}

/// An image as the model reads it: JPEG, at most [`IMAGE_MAX_SIDE`] pixels
/// on its longest side (WhatsApp stickers are WEBP).
fn model_image(bytes: &[u8]) -> Option<String> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width().max(image.height()) > IMAGE_MAX_SIDE {
        image.resize(IMAGE_MAX_SIDE, IMAGE_MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut jpeg), IMAGE_QUALITY).encode_image(&image.to_rgb8()).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(jpeg))
}

// ---------------------------------------------------------------------------
// Agent tools
// ---------------------------------------------------------------------------

fn connected_link(app: &AppHandle, library_id: &str) -> Result<Arc<Link>, BackendError> {
    if !is_supported(app) {
        return Err(BackendError::new(BackendErrorCode::Unsupported, unsupported_reason(), false));
    }
    link_for(app, library_id).filter(|link| lock(&link.status).state == LinkState::Open).ok_or_else(not_connected)
}

/// The chat a command names, among those that allow what it needs, and
/// whether it is a number nobody wrote to yet (only to send to it).
fn command_chat(link: &Link, command: &WhatsAppCommand) -> Result<Option<(String, bool)>, BackendError> {
    let Some(wanted) = command.chat() else { return Ok(None) };
    let config = link.config();
    let store = lock(&link.store);
    match core::resolve_chat(&store, &config, wanted, command.access()) {
        Ok(chat) => Ok(Some((chat, false))),
        Err(error) if command.access() == ChatAccess::Write && error.code == BackendErrorCode::InvalidInput => {
            core::new_number_chat(&store, &config, wanted).map(|chat| Some((chat, true))).ok_or(error)
        }
        Err(error) => Err(error),
    }
}

/// The confirmation of a message or a file: the chat and exactly what
/// goes. A chat that is not allowed goes back to the model before asking.
pub(crate) fn preview(app: &AppHandle, context: &BackendRequestContext, name: &str, arguments: &Value) -> Result<(String, String), BackendError> {
    let link = connected_link(app, &context.library_id)?;
    let command = WhatsAppCommand::from_call(name, arguments)?;
    let (chat, new) = command_chat(&link, &command)?.unwrap_or_default();
    let chat_name = core::name_of(&lock(&link.store), &link.config(), &chat);
    let mut detail = command.detail(&chat_name, &chat);
    if new {
        detail.push_str("\n\n");
        detail.push_str(core::NEW_CHAT_NOTE);
    }
    Ok((command.summary(&chat_name), detail))
}

pub(crate) fn execute_tool(app: &AppHandle, context: &BackendRequestContext, name: &str, arguments: &Value) -> Result<Value, BackendError> {
    let link = connected_link(app, &context.library_id)?;
    let command = WhatsAppCommand::from_call(name, arguments)?;
    let (chat, new) = command_chat(&link, &command)?.unwrap_or_default();
    let result = match &command {
        WhatsAppCommand::ListChats { query, limit } => core::list_chats_result(&lock(&link.store), &link.config(), query.as_deref(), *limit, &local_time),
        WhatsAppCommand::ReadMessages { limit, before, .. } => {
            core::messages_result(&lock(&link.store), &link.config(), &chat, *limit, before.as_deref(), &local_time)?
        }
        WhatsAppCommand::GetImage { message_id, .. } => {
            let message = core::image_message(&lock(&link.store), &chat, message_id)?.clone();
            let image = link.download_image(&message.id)?;
            core::image_result(&lock(&link.store), &link.config(), &chat, &message, image, &local_time)
        }
        WhatsAppCommand::SendMessage { text, reply_to: Some(menu_id), .. } => {
            let option = core::menu_option(&lock(&link.store), &chat, menu_id, text)?;
            let (chat, id) = link.send_option(&chat, menu_id, &option)?.unwrap_or((chat, String::new()));
            core::sent_result(&lock(&link.store), &link.config(), &chat, &id, None)
        }
        WhatsAppCommand::SendMessage { text, reply_to: None, .. } => {
            let (chat, id) = link.send_text(&chat, text, new)?.unwrap_or((chat, String::new()));
            if new {
                link.allow_new_chat(app, &chat)?;
            }
            core::sent_result(&lock(&link.store), &link.config(), &chat, &id, None)
        }
        WhatsAppCommand::SendFile { path, caption, .. } => {
            let bytes = library_file(app, &context.library_id, path)?;
            let (media_type, as_image) = core::file_media(path);
            let file_name = path.rsplit('/').next().unwrap_or(path);
            let answer = link.request(
                json!({
                    "op": "sendFile",
                    "chat": chat,
                    "base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
                    "fileName": file_name,
                    "mimetype": media_type,
                    "caption": caption,
                    "asImage": as_image,
                    "lookup": new,
                }),
                FILE_ANSWER_TIMEOUT,
            )?;
            let (chat, id) = link.keep_sent(&answer).unwrap_or((chat, String::new()));
            if new {
                link.allow_new_chat(app, &chat)?;
            }
            core::sent_result(&lock(&link.store), &link.config(), &chat, &id, Some(path))
        }
    };
    // Only the step is logged: chats and messages are private.
    log::info!("[notia:whatsapp] herramienta {name} hecha");
    Ok(result)
}

// ---------------------------------------------------------------------------
// Waits of AI tasks
// ---------------------------------------------------------------------------

/// The chat an AI task waits for a reply in (it needs Lectura), its name
/// and where the replies start: at the message the task just sent there.
pub(crate) fn wait_target(app: &AppHandle, library_id: &str, wanted: &str, run_started_ms: i64) -> Result<(String, String, i64), BackendError> {
    let link = connected_link(app, library_id)?;
    let config = link.config();
    let store = lock(&link.store);
    let chat = core::resolve_chat(&store, &config, wanted, ChatAccess::Read)?;
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let since = core::wait_since(store.messages(&chat), now, run_started_ms.max(0) as u64);
    Ok((chat.clone(), core::name_of(&store, &config, &chat), since as i64))
}

/// The note that wakes the task once they replied in `chat` and stopped
/// writing for [`crate::backend::reply_wait::WHATSAPP_TASK_WAIT`]; `None` while
/// nothing arrived. Only the store is read: nothing goes to WhatsApp.
pub(crate) fn wait_replies(app: &AppHandle, library_id: &str, chat: &str, since_ms: i64, now_ms: i64) -> Result<Option<String>, BackendError> {
    let link = connected_link(app, library_id)?;
    let config = link.config();
    if !config.chat(chat).read {
        return Err(BackendError::invalid_input("El chat ya no tiene permiso de Lectura: la persona lo habilita en Configuración → WhatsApp."));
    }
    let store = lock(&link.store);
    let quiet = crate::backend::reply_wait::WHATSAPP_TASK_WAIT.as_millis() as u64;
    let replies = core::replies_after(&store, chat, since_ms.max(0) as u64, quiet, now_ms.max(0) as u64);
    Ok(replies.map(|replies| core::replies_note(&store, &config, chat, &replies, &local_time)))
}

fn library_file(app: &AppHandle, library_id: &str, path: &str) -> Result<Vec<u8>, BackendError> {
    let bytes = crate::library_documents::with_documents(app, library_id, |documents| {
        let locator = documents.locator(path)?;
        if !documents.adapter.exists_locator(&locator)? {
            return Err(BackendError::invalid_input(format!("No existe el archivo {path} en la biblioteca.")));
        }
        documents.adapter.read_binary_locator(&locator)
    })?;
    if bytes.len() > core::MAX_FILE_BYTES {
        return Err(BackendError::invalid_input("El archivo pesa más de 16 MB: WhatsApp lo recibe, pero el asistente manda hasta ese tamaño."));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Settings commands
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhatsAppPayload {
    library_id: String,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
    #[serde(default)]
    auto_reply: bool,
    #[serde(default)]
    prompt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhatsAppView {
    /// This device links WhatsApp (a desktop host).
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<&'static str>,
    enabled: bool,
    /// `off` while WhatsApp is not on; else the link's state.
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_svg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<AccountView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    chats: Vec<ChatView>,
    total_chats: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountView {
    name: Option<String>,
    number: Option<String>,
}

async fn blocking<T: Send + 'static>(task: impl FnOnce() -> Result<T, BackendError> + Send + 'static) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| BackendError::new(BackendErrorCode::Internal, "No se pudo completar la operación de WhatsApp.", true))?
}

fn state_id(state: LinkState) -> &'static str {
    match state {
        LinkState::Starting => "starting",
        LinkState::Qr => "qr",
        LinkState::Open => "open",
        LinkState::Closed => "closed",
        LinkState::Error => "error",
    }
}

fn view(app: &AppHandle, library_id: &str, query: &str) -> Result<WhatsAppView, BackendError> {
    let available = is_supported(app);
    let config = read_config(app, library_id)?;
    let link = link_for(app, library_id);
    let status = link.as_ref().map(|link| lock(&link.status).clone());
    let notice = lock(&app.state::<WhatsAppState>().notice).as_ref().filter(|(library, _)| library == library_id).map(|(_, notice)| notice.clone());
    let (state, message) = match (&status, config.enabled) {
        (Some(status), true) => (state_id(status.state), status.message.clone()),
        // On, and the supervisor has not opened the link yet.
        (None, true) if available => ("starting", Some("Preparando WhatsApp…".to_string())),
        _ => ("off", notice),
    };
    let (chats, total_chats) = match &link {
        Some(link) => core::settings_chats(&lock(&link.store), &config, query, SETTINGS_CHAT_LIMIT),
        // Off: the chats configured before (the store went with the session).
        None => core::settings_chats(&WhatsAppStore::default(), &config, query, SETTINGS_CHAT_LIMIT),
    };
    let account = status.as_ref().filter(|status| status.state == LinkState::Open).map(|status| AccountView {
        name: status.account_name.clone(),
        number: status.account_id.as_deref().and_then(|id| id.split_once('@')).filter(|(_, server)| *server == "s.whatsapp.net").map(|(number, _)| format!("+{number}")),
    });
    Ok(WhatsAppView {
        available,
        unavailable_reason: (!available).then(|| unsupported_reason()),
        enabled: config.enabled,
        state,
        qr_svg: status.as_ref().filter(|status| status.state == LinkState::Qr).and_then(|status| status.qr_svg.clone()),
        account,
        message,
        chats,
        total_chats,
    })
}

/// The link's state, its QR while it waits for one and the chats with
/// their settings that match `query`.
pub(crate) async fn whatsapp_view(app: AppHandle, payload: WhatsAppPayload) -> Result<WhatsAppView, BackendError> {
    blocking(move || view(&app, &payload.library_id, payload.query.as_deref().unwrap_or_default())).await
}

/// Turns WhatsApp on: the supervisor starts the link and the settings
/// show its QR.
pub(crate) async fn whatsapp_connect(app: AppHandle, payload: WhatsAppPayload) -> Result<WhatsAppView, BackendError> {
    blocking(move || {
        if !is_supported(&app) {
            return Err(BackendError::new(BackendErrorCode::Unsupported, unsupported_reason(), false));
        }
        let mut config = read_config(&app, &payload.library_id)?;
        config.enabled = true;
        save_config(&app, &payload.library_id, &config)?;
        lock(&app.state::<WhatsAppState>().notice).take();
        // A link that ended, or could not start, opens again right away.
        let state = app.state::<WhatsAppState>();
        let mut current = lock(&state.link);
        if current.as_ref().is_some_and(|link| link.stopped.load(Ordering::SeqCst) || lock(&link.status).state == LinkState::Error) {
            if let Some(previous) = current.take() {
                previous.stop();
            }
        }
        drop(current);
        lock(&state.decision).take();
        reconcile(&app);
        view(&app, &payload.library_id, payload.query.as_deref().unwrap_or_default())
    })
    .await
}

/// Unlinks Notia from WhatsApp and deletes the session and the stored
/// chats of this PC. The chats' settings stay in the configuration.
pub(crate) async fn whatsapp_disconnect(app: AppHandle, payload: WhatsAppPayload) -> Result<WhatsAppView, BackendError> {
    blocking(move || {
        let state = app.state::<WhatsAppState>();
        let link = {
            // Off before the link leaves, under the supervisor's lock.
            let mut current = lock(&state.link);
            let mut config = read_config(&app, &payload.library_id)?;
            config.enabled = false;
            save_config(&app, &payload.library_id, &config)?;
            if current.as_ref().is_some_and(|link| link.library_id == payload.library_id) {
                current.take()
            } else {
                None
            }
        };
        if let Some(link) = link {
            // The phone drops the device too; offline, it notices later.
            let _ = link.request(json!({ "op": "logout" }), LOGOUT_TIMEOUT);
            link.stop();
        }
        lock(&state.notice).take();
        lock(&state.decision).take();
        if let Some(folder) = data_folder(&app, &payload.library_id) {
            remove_folder(&folder);
        }
        view(&app, &payload.library_id, payload.query.as_deref().unwrap_or_default())
    })
    .await
}

fn remove_folder(folder: &Path) {
    if folder.exists() && std::fs::remove_dir_all(folder).is_err() {
        log::error!("[notia:whatsapp] no se pudo borrar la sesión guardada");
    }
}

/// Saves what the assistant may do in one chat and its auto reply.
pub(crate) async fn whatsapp_set_chat(app: AppHandle, payload: WhatsAppPayload) -> Result<WhatsAppView, BackendError> {
    blocking(move || {
        let chat = payload.chat_id.as_deref().map(str::trim).filter(|chat| core::is_chat_id(chat)).ok_or_else(|| BackendError::invalid_input("El chat de WhatsApp no es válido."))?;
        let mut config = read_config(&app, &payload.library_id)?;
        let link = link_for(&app, &payload.library_id);
        let name = match &link {
            Some(link) => core::name_of(&lock(&link.store), &config, chat),
            None => config.chat(chat).name,
        };
        config.set_chat(ChatSettings {
            id: chat.to_string(),
            name,
            read: payload.read,
            write: payload.write,
            auto_reply: payload.auto_reply,
            prompt: payload.prompt.clone(),
        })?;
        save_config(&app, &payload.library_id, &config)?;
        if let Some(link) = &link {
            if !payload.auto_reply {
                lock(&link.replies).remove(chat);
            }
            *lock(&link.config) = config;
        }
        view(&app, &payload.library_id, payload.query.as_deref().unwrap_or_default())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_reaches_the_model_as_a_jpeg_of_bounded_size() {
        let encode = |width, height| {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(Cursor::new(&mut png))
                .write_image(&vec![0; (width * height * 3) as usize], width, height, image::ExtendedColorType::Rgb8)
                .expect("png");
            png
        };
        use image::ImageEncoder as _;
        let decoded = |base64: String| image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(base64).expect("base64")).expect("image");
        let large = decoded(model_image(&encode(3200, 1600)).expect("large"));
        assert_eq!((large.width(), large.height()), (1600, 800));
        let small = decoded(model_image(&encode(300, 200)).expect("small"));
        assert_eq!((small.width(), small.height()), (300, 200));
        assert!(model_image(b"no es una imagen").is_none());
    }

    #[test]
    fn a_moment_is_written_in_local_time() {
        assert_eq!(local_time(0).len(), "1970-01-01 00:00".len());
        assert!(local_time(u64::MAX).is_empty());
    }

    /// Installs Baileys in a temporary folder (npm and the network) and runs
    /// the real script until WhatsApp offers a QR to link Notia:
    /// `cargo test -p notia-app whatsapp -- --ignored --exact whatsapp::tests::the_script_connects_and_offers_a_qr`.
    #[test]
    #[ignore = "needs Node.js 20, npm and the network"]
    fn the_script_connects_and_offers_a_qr() {
        let package = std::env::temp_dir().join(format!("notia-whatsapp-{}", std::process::id()));
        AGENT_PACKAGE.install_into(&package).expect("install");
        let data = package.join("data");
        let mut child = Command::new("node")
            .arg(package.join(AGENT_PACKAGE.script_name))
            .arg(&data)
            .current_dir(&package)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("node");
        let mut stdin = child.stdin.take().expect("stdin");
        let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut qr = None;
        while qr.is_none() && Instant::now() < deadline {
            let Some(Ok(line)) = lines.next() else { break };
            let value: Value = serde_json::from_str(&line).expect("a JSON line");
            if value["event"] == "qr" {
                qr = value["svg"].as_str().map(str::to_string);
            }
        }
        assert!(qr.is_some_and(|svg| svg.starts_with("<svg")), "no QR");
        // Before the link, a send is refused with a reason.
        writeln!(stdin, "{}", json!({ "id": 1, "op": "send", "chat": "5491111111111@s.whatsapp.net", "text": "x" })).expect("write");
        let answer = lines.find_map(|line| serde_json::from_str::<Value>(&line.ok()?).ok().filter(|value| value["id"] == 1)).expect("answer");
        assert_eq!(answer["ok"], false);
        drop(stdin);
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&package);
    }

    #[test]
    fn the_wire_messages_of_the_script_fit_the_store() {
        let line = json!({
            "event": "messages",
            "live": true,
            "messages": [{ "chat": "5491111111111@s.whatsapp.net", "id": "ABC", "fromMe": false, "senderName": "Juan", "timestampMs": 1_000, "text": "hola", "kind": "text" }],
        });
        let messages = serde_json::from_value::<Vec<WireMessage>>(line["messages"].clone()).expect("wire");
        let mut store = WhatsAppStore::default();
        let added = store.add_messages(messages);
        assert_eq!(added.len(), 1);
        assert_eq!(store.display_name("5491111111111@s.whatsapp.net"), "Juan");
        let chats = serde_json::from_value::<Vec<ChatUpdate>>(json!([{ "id": "120363000000000001@g.us", "name": "Familia", "timestampMs": 2_000, "unread": 3 }])).expect("chats");
        store.upsert_chats(chats);
        assert_eq!(store.display_name("120363000000000001@g.us"), "Familia");
    }
}
