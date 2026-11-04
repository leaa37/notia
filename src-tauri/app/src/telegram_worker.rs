//! Telegram bot of the selected library, run by the backend so it keeps
//! working while the WebView is hidden, reloaded or closed to the tray.
//!
//! A supervisor starts one worker per (library, bot token) from the library
//! configuration. The worker polls Telegram on one thread and runs queued
//! requests on another through the Rust agent runtime; confirmations,
//! clarifications and plans pause the run and resume it with the user's
//! answer. A message sent while the chat's request runs goes through a short
//! parallel call to the model that decides whether it stops that request or
//! waits in the queue (see `backend_core::turn_interrupts`). Photos, images
//! and PDFs go to the library chat like text: the model decides what kind of
//! request they are (see `backend_core::tool_routing`), and the parts of an
//! album become one request. A new request waits while the person may still
//! be writing (see `backend_core::reply_wait`): what the chat sends within
//! ten seconds of its last message joins it. Offsets, processed
//! updates and the queue survive restarts; the text of queued requests is
//! never stored.
//!
//! The autonomous agent (`agent_autonomy`) also runs here, as requests of
//! the Owner's chat that Notia queues by itself: they only read, show no
//! progress, are never stored, send their answer only when it is not
//! silence, and give way as soon as the Owner writes.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::host::{AppHandle, Emitter, Manager};

use crate::backend::agent_autonomy::{self as autonomy, AutonomousKind};
use crate::backend::telegram_bot as bot;
use crate::backend::{
    AgentRequest, BackendActor, BackendChannel, BackendMessage, BackendRequest, BackendRequestContext,
    BackendRequestEnvelope, BackendResponse, BackendScope, MessageRole, PersistencePolicy,
    ProtocolVersion, ResumeDecision, ResumeRequest,
};
use crate::backend::chat_attachments::MessageAttachment;
use crate::library_catalog::CatalogLibrary;
use crate::library_users::LibraryDatabaseContext;
use crate::services::telegram_service::{self as telegram, IncomingTelegramUpdate, TelegramDocument, TelegramPhoto};

const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(5);
const POLL_RETRY_DELAY: Duration = Duration::from_secs(3);
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(120);
const CLARIFICATION_TIMEOUT: Duration = Duration::from_secs(600);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);
const LINK_TIMEOUT: Duration = Duration::from_secs(120);
const LINK_MAX_ATTEMPTS: u32 = 5;
const LINK_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_PROCESSED_UPDATES: usize = 200;
/// A cancel sent before the run registered its control is tried again.
const CANCEL_RETRY_DELAY: Duration = Duration::from_millis(500);
const CANCEL_ATTEMPTS: u32 = 120;
/// The parts of an album arrive as separate updates, usually within a
/// second; the album is sent once no part arrived for this long.
const ALBUM_WINDOW: Duration = Duration::from_millis(1500);
const PART_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How often a batch that waits for more messages checks its time.
const BATCH_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Telegram albums hold up to 10 photos or files, and so does a request.
const MAX_ALBUM_PARTS: usize = 10;
const OWNER: &str = "user-owner";
/// Event emitted to the interface after a Telegram request changed data.
pub(crate) const LIBRARY_CHANGED_EVENT: &str = "notia://telegram-library-changed";

#[derive(Default)]
pub(crate) struct TelegramWorkerState {
    running: Mutex<Option<RunningWorker>>,
}

struct RunningWorker {
    key: WorkerKey,
    stop: Arc<AtomicBool>,
    worker: Arc<Worker>,
}

/// What happened to a run the autonomous agent asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutonomousEnqueue {
    Queued,
    /// Another autonomous run is still queued or running.
    Busy,
    /// No bot runs for the library, or its Owner has not linked Telegram.
    Unavailable,
}

fn running_worker(app: &AppHandle, library_id: &str) -> Option<Arc<Worker>> {
    let state = app.state::<TelegramWorkerState>();
    let running = state.running.lock().ok()?;
    running
        .as_ref()
        .filter(|running| running.key.library_id == library_id && !running.worker.stopped())
        .map(|running| Arc::clone(&running.worker))
}

/// Whether the bot of `library_id` runs on this device, the only place its
/// autonomous agent can write from.
pub(crate) fn bot_runs_for(app: &AppHandle, library_id: &str) -> bool {
    running_worker(app, library_id).is_some()
}

/// Sends a message of an AI task (Markdown) to `chat_id`, the Owner's chat
/// of `library_id`, with `buttons` on its last part; that part's id.
/// `None` when no bot runs for the library here.
pub(crate) fn send_task_message(app: &AppHandle, library_id: &str, chat_id: i64, markdown: &str, buttons: Vec<(String, String)>) -> Option<i64> {
    let worker = running_worker(app, library_id)?;
    worker.send_parts(chat_id, bot::telegram_html_parts(markdown), buttons).map(|(id, _)| id)
}

/// Rewrites a message of an AI task (HTML), which loses its buttons.
pub(crate) fn mark_task_message(app: &AppHandle, library_id: &str, chat_id: i64, message_id: i64, html: &str) {
    if let Some(worker) = running_worker(app, library_id) {
        worker.edit_html(chat_id, message_id, html);
    }
}

/// Queues a run of the autonomous agent in the Owner's Telegram chat of
/// `library_id`, with `trigger` as its request.
pub(crate) fn enqueue_autonomous(app: &AppHandle, library_id: &str, kind: AutonomousKind, trigger: String) -> AutonomousEnqueue {
    match running_worker(app, library_id) {
        Some(worker) => worker.enqueue_autonomous(kind, trigger),
        None => AutonomousEnqueue::Unavailable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerKey {
    library_id: String,
    token: String,
}

/// Plugin that starts the supervisor keeping the worker in line with the
/// selected library and its Telegram configuration.
pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("telegram-worker")
        .setup(|app, _api| {
            let app = app.clone();
            let _ = std::thread::Builder::new().name("notia-telegram-supervisor".into()).spawn(move || loop {
                std::thread::sleep(SUPERVISOR_INTERVAL);
                reconcile(&app);
            });
            Ok(())
        })
        .build()
}

/// What the supervisor decided last, and from what. Reading the
/// configuration means opening and decrypting a file, through the slow
/// storage of Android; the decision is taken again when Notia writes a
/// configuration, a library is locked or unlocked, the selected library
/// changes, or after [`DECISION_TTL`] (a change made outside Notia).
struct Decision {
    generation: u64,
    library: Option<String>,
    client: bool,
    at: std::time::Instant,
    worker: Option<(WorkerKey, CatalogLibrary)>,
}

const DECISION_TTL: Duration = Duration::from_secs(60);
static DECISION: Mutex<Option<Decision>> = Mutex::new(None);

fn desired_worker(app: &AppHandle) -> Option<(WorkerKey, CatalogLibrary)> {
    let generation = crate::library_config::config_generation();
    let library = crate::library_catalog::selected_library_id(app);
    let client = !crate::device_role::runs_services(app);
    if let Ok(last) = DECISION.lock() {
        if let Some(last) = last.as_ref().filter(|last| {
            last.generation == generation && last.library == library && last.client == client && last.at.elapsed() < DECISION_TTL
        }) {
            return last.worker.clone();
        }
    }
    let worker = read_desired_worker(app, client);
    if let Ok(mut last) = DECISION.lock() {
        *last = Some(Decision { generation, library, client, at: std::time::Instant::now(), worker: worker.clone() });
    }
    worker
}

fn read_desired_worker(app: &AppHandle, client: bool) -> Option<(WorkerKey, CatalogLibrary)> {
    // Only the computer that runs the library's services answers Telegram.
    if client {
        return None;
    }
    let library = crate::library_catalog::selected_library(app)?;
    app.state::<crate::library_registry::LibraryBindingRegistry>().lookup(&library.id).ok()?;
    let config = crate::library_config::read_library_config(app, &library.id).ok()??;
    let section = config.get("telegram")?;
    let token = section.get("botToken").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if section.get("enabled").and_then(Value::as_bool) != Some(true) || token.is_empty() {
        return None;
    }
    Some((WorkerKey { library_id: library.id.clone(), token }, library))
}

fn reconcile(app: &AppHandle) {
    let desired = desired_worker(app);
    let state = app.state::<TelegramWorkerState>();
    let Ok(mut running) = state.running.lock() else {
        return;
    };
    if running.as_ref().map(|running| &running.key) == desired.as_ref().map(|(key, _)| key) {
        return;
    }
    if let Some(previous) = running.take() {
        previous.stop.store(true, Ordering::SeqCst);
    }
    if let Some((key, library)) = desired {
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(worker) = Worker::start(app.clone(), library, key.token.clone(), Arc::clone(&stop)) {
            *running = Some(RunningWorker { key, stop, worker });
        }
    }
}

// ---------------------------------------------------------------------------
// Persisted state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
enum JobAttachment {
    Photo(TelegramPhoto),
    /// A PDF or an image sent as a file (`telegram::document_kind`).
    #[serde(alias = "pdf")]
    Document(TelegramDocument),
}

/// A queued request as stored on disk: without its text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredJob {
    request_id: String,
    chat_id: i64,
    telegram_user_id: i64,
    library_user_id: String,
    /// Files of the request: a photo, a document or the parts of an album.
    #[serde(default)]
    attachments: Vec<JobAttachment>,
    /// The single file older queues stored; read into `attachments`.
    #[serde(default, skip_serializing)]
    attachment: Option<JobAttachment>,
}

impl StoredJob {
    fn adopt_legacy_attachment(&mut self) {
        if let Some(attachment) = self.attachment.take() {
            self.attachments.insert(0, attachment);
        }
    }
}

/// Parts of an album that are still arriving; they become one request.
struct PendingAlbum {
    telegram_user_id: i64,
    library_user_id: String,
    text: String,
    attachments: Vec<JobAttachment>,
    last_part: Instant,
    /// The album joins the chat's batch (a new request) instead of going
    /// on at once (a message for a running request or a question).
    batched: bool,
}

/// What a chat sent while the person may still be writing: the messages
/// that arrive meanwhile join it, and it goes on as one request once none
/// arrived for the wait of the last one (see `backend_core::reply_wait`).
struct PendingBatch {
    /// The request it becomes, stored without its text (a restart reports
    /// it as interrupted).
    request_id: String,
    telegram_user_id: i64,
    library_user_id: String,
    parts: Vec<String>,
    attachments: Vec<JobAttachment>,
    ready_at: Instant,
}

impl PendingBatch {
    /// One message per line, in the order they arrived.
    fn text(&self) -> String {
        self.parts.join("\n")
    }

    fn stored(&self, chat_id: i64) -> StoredJob {
        StoredJob {
            request_id: self.request_id.clone(),
            chat_id,
            telegram_user_id: self.telegram_user_id,
            library_user_id: self.library_user_id.clone(),
            attachments: self.attachments.clone(),
            attachment: None,
        }
    }
}

/// Adds a message to the batch of its chat, which then waits `wait` from
/// now; `true` when the message opened the batch.
fn add_to_batch(
    batches: &mut HashMap<i64, PendingBatch>,
    chat_id: i64,
    telegram_user_id: i64,
    library_user_id: String,
    text: String,
    attachments: Vec<JobAttachment>,
    wait: Duration,
) -> bool {
    let opened = !batches.contains_key(&chat_id);
    let batch = batches.entry(chat_id).or_insert_with(|| PendingBatch {
        request_id: short_id(),
        telegram_user_id,
        library_user_id,
        parts: Vec::new(),
        attachments: Vec::new(),
        ready_at: Instant::now(),
    });
    if !text.trim().is_empty() {
        batch.parts.push(text);
    }
    let room = MAX_ALBUM_PARTS.saturating_sub(batch.attachments.len());
    batch.attachments.extend(attachments.into_iter().take(room));
    batch.ready_at = Instant::now() + wait;
    opened
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerFile {
    offset: i64,
    #[serde(default)]
    processed: VecDeque<i64>,
    #[serde(default)]
    jobs: Vec<StoredJob>,
}

#[derive(Debug, Clone)]
struct Job {
    stored: StoredJob,
    text: String,
    /// Set when Notia queued the run by itself; such a job is never stored.
    autonomous: Option<AutonomousKind>,
    /// Set for the run of an AI action; never stored either: an action left
    /// in the queue by a restart is marked interrupted by `ai_actions`.
    action: Option<ActionJob>,
    /// Set for the run of a webhook call; never stored: a program calls
    /// again if Notia restarted.
    webhook: Option<WebhookJob>,
}

impl Job {
    /// Only the requests people sent survive a restart.
    fn is_persisted(&self) -> bool {
        self.autonomous.is_none() && self.action.is_none() && self.webhook.is_none()
    }
}

/// The run of an AI action (see `ai_actions`): a turn of the Owner's chat
/// whose request is the action's prompt.
#[derive(Debug, Clone)]
pub(crate) struct ActionJob {
    pub(crate) run_id: String,
    pub(crate) prompt: crate::backend::ai_actions::prompts::ScheduledActionPrompt,
    pub(crate) request: String,
}

/// What happened to the run an action asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionEnqueue {
    Queued,
    /// No bot runs for the library here, or its Owner has not linked Telegram.
    Unavailable,
}

/// Queues the run of an AI action in the Owner's Telegram chat of
/// `library_id`.
pub(crate) fn enqueue_action(app: &AppHandle, library_id: &str, job: ActionJob) -> ActionEnqueue {
    match running_worker(app, library_id) {
        Some(worker) => worker.enqueue_owner_run(job.request.clone(), Some(job), None),
        None => ActionEnqueue::Unavailable,
    }
}

/// The run of a webhook call (see `webhooks`): a turn of the Owner's chat
/// whose request is the prompt another program sent.
#[derive(Debug, Clone)]
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(crate) struct WebhookJob {
    pub(crate) call_id: String,
    pub(crate) prompt: crate::backend::webhooks::WebhookCallPrompt,
    pub(crate) request: String,
}

/// Queues the run of a webhook call in the Owner's Telegram chat of
/// `library_id`.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(crate) fn enqueue_webhook(app: &AppHandle, library_id: &str, job: WebhookJob) -> ActionEnqueue {
    match running_worker(app, library_id) {
        Some(worker) => worker.enqueue_owner_run(job.request.clone(), None, Some(job)),
        None => ActionEnqueue::Unavailable,
    }
}

/// What a turn Notia started for the Owner answered, and the messages
/// already sent for it (notes before a question, images it showed).
struct UnattendedAnswer {
    answer: String,
    request_id: String,
    notes: Vec<i64>,
    shown: Vec<MessageAttachment>,
}

/// The request running now, so a message of its chat can stop it. Its text
/// stays in memory only, for the decision about new messages.
struct CurrentRun {
    chat_id: i64,
    text: String,
    context: BackendRequestContext,
    idempotency_key: String,
    cancelled: Arc<AtomicBool>,
    autonomous: bool,
}

/// Identity of a job's run while it goes on.
struct ActiveRun {
    context: BackendRequestContext,
    idempotency_key: String,
    cancelled: Arc<AtomicBool>,
}

/// An answer to a confirmation, plan or question; the flag says it came
/// from a button of the question's message, not from a typed message.
enum Reply {
    Decision(bot::ConfirmationReply, bool),
    Text(String, Option<usize>, bool),
}

/// What the person answered, for the run to go on.
struct Answer {
    decision: ResumeDecision,
    /// What the progress says first after the answer.
    acknowledgement: Option<String>,
    /// The question's message, answered with a button, and its text
    /// (HTML): the progress goes on in it, below the answer.
    message: Option<(i64, String)>,
}

#[derive(Debug, Clone)]
enum Prompt {
    Confirmation { id: String },
    Clarification { choices: Vec<String> },
    /// The person chose «Proponer otra cosa» and writes the proposal.
    Proposal,
}

#[derive(Debug, Clone)]
enum LinkStep {
    Username,
    NewPassword,
    ConfirmPassword(String),
    ExistingPassword,
}

#[derive(Debug, Clone)]
struct LinkFlow {
    step: LinkStep,
    user_id: Option<String>,
    expires_at: Instant,
    attempts: u32,
    blocked_until: Option<Instant>,
}

struct Worker {
    app: AppHandle,
    library: CatalogLibrary,
    token: String,
    stop: Arc<AtomicBool>,
    file: PathBuf,
    persisted: Mutex<WorkerFile>,
    queue: Mutex<VecDeque<Job>>,
    queue_ready: Condvar,
    interrupted: Mutex<Vec<StoredJob>>,
    active: Mutex<Option<StoredJob>>,
    current: Mutex<Option<CurrentRun>>,
    prompt: Mutex<Option<(i64, Prompt, mpsc::Sender<Reply>)>>,
    history: Mutex<HashMap<i64, VecDeque<BackendMessage>>>,
    links: Mutex<HashMap<i64, LinkFlow>>,
    albums: Mutex<HashMap<(i64, String), PendingAlbum>>,
    batches: Mutex<HashMap<i64, PendingBatch>>,
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    crate::host::async_runtime::block_on(future)
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..24].to_string()
}

impl Worker {
    fn start(app: AppHandle, library: CatalogLibrary, token: String, stop: Arc<AtomicBool>) -> Option<Arc<Worker>> {
        let bot_id = token.split(':').next().unwrap_or("bot").chars().filter(char::is_ascii_digit).collect::<String>();
        let library_key = library.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect::<String>();
        let directory = app.path().app_data_dir().map(|directory| directory.join("telegram")).ok()?;
        let file = directory.join(format!("{library_key}-{bot_id}.json"));
        let mut persisted = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<WorkerFile>(&text).ok())
            .unwrap_or_default();
        persisted.jobs.iter_mut().for_each(StoredJob::adopt_legacy_attachment);
        let worker = Arc::new(Worker {
            app,
            library,
            token,
            stop,
            file,
            interrupted: Mutex::new(persisted.jobs.clone()),
            persisted: Mutex::new(WorkerFile { jobs: Vec::new(), ..persisted }),
            queue: Mutex::new(VecDeque::new()),
            queue_ready: Condvar::new(),
            active: Mutex::new(None),
            current: Mutex::new(None),
            prompt: Mutex::new(None),
            history: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
            albums: Mutex::new(HashMap::new()),
            batches: Mutex::new(HashMap::new()),
        });
        worker.persist();
        worker.notify_interrupted();
        let executor = Arc::clone(&worker);
        let _ = std::thread::Builder::new().name("notia-telegram-executor".into()).spawn(move || executor.run_queue());
        let poller = Arc::clone(&worker);
        let _ = std::thread::Builder::new().name("notia-telegram-poller".into()).spawn(move || poller.poll());
        Some(worker)
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn persist(&self) {
        let Ok(mut persisted) = self.persisted.lock() else {
            return;
        };
        let mut jobs = self.active.lock().map(|active| active.iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
        jobs.extend(self.interrupted.lock().map(|items| items.clone()).unwrap_or_default());
        jobs.extend(
            self.queue
                .lock()
                .map(|queue| queue.iter().filter(|job| job.is_persisted()).map(|job| job.stored.clone()).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        jobs.extend(
            self.batches
                .lock()
                .map(|batches| batches.iter().map(|(chat_id, batch)| batch.stored(*chat_id)).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        persisted.jobs = jobs;
        let Ok(text) = serde_json::to_string(&*persisted) else {
            return;
        };
        if let Some(parent) = self.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temporary = self.file.with_extension("json.tmp");
        if std::fs::write(&temporary, text).is_ok() {
            let _ = std::fs::rename(&temporary, &self.file);
        }
    }

    fn send(&self, chat_id: i64, text: &str) -> Option<i64> {
        block_on(telegram::send_message(&self.token, chat_id, text, Vec::new(), None)).ok()
    }

    fn send_html(&self, chat_id: i64, html: &str, buttons: Vec<(String, String)>) -> Option<i64> {
        match block_on(telegram::send_message(&self.token, chat_id, html, buttons.clone(), Some("HTML"))) {
            Ok(id) => Some(id),
            // Telegram rejects malformed HTML; the text still reaches the user.
            Err(_) => block_on(telegram::send_message(&self.token, chat_id, &strip_html(html), buttons, None)).ok(),
        }
    }

    /// Sends the images the agent made in `request_id` as photos of the
    /// chat, and returns the ones that arrived as attachments for the chat's
    /// history. Nothing is saved in the library: Telegram is ephemeral.
    fn send_generated_images(&self, chat_id: i64, request_id: &str) -> Vec<MessageAttachment> {
        use crate::backend::chat_attachments::MessageAttachmentKind;
        let mut sent = Vec::new();
        for image in crate::image_generation::take_generated(request_id) {
            let Ok(png) = base64::engine::general_purpose::STANDARD.decode(&image.base64) else { continue };
            match block_on(telegram::send_photo(&self.token, chat_id, &image.name, png)) {
                Ok(_) => sent.push(MessageAttachment {
                    name: image.name,
                    media_type: image.media_type.to_string(),
                    kind: MessageAttachmentKind::Image,
                    pages: vec![image.base64],
                    text_content: None,
                    extracted_text: None,
                    page_count: None,
                }),
                Err(_) => {
                    log::error!("[notia:telegram] no se pudo enviar una imagen generada");
                    self.send(chat_id, "No pude enviar la imagen que generé; pedímela de nuevo.");
                }
            }
        }
        sent
    }

    /// Sends a Markdown text as one or more messages, split between
    /// paragraphs so a long answer is neither cut nor loses its format.
    fn send_markdown(&self, chat_id: i64, markdown: &str) {
        self.send_parts(chat_id, bot::telegram_html_parts(markdown), Vec::new());
    }

    /// Before a question, sends whole the agent's notes the person has not
    /// read since the last delivery (see `bot::notes_to_deliver`): the
    /// question's context. Returns the messages sent, which go once the
    /// answer arrives (`delete_messages`).
    fn deliver_notes(
        &self,
        chat_id: i64,
        runtime: &crate::backend_runtime::BackendRuntimeState,
        context: &BackendRequestContext,
        seen_events: &mut u64,
    ) -> Vec<i64> {
        let events = runtime.request_events(context, *seen_events).unwrap_or_default();
        *seen_events = events.iter().map(|envelope| envelope.sequence).max().unwrap_or(*seen_events);
        let events = events.into_iter().map(|envelope| envelope.event).collect::<Vec<_>>();
        let mut sent = Vec::new();
        for note in bot::notes_to_deliver(&events, true) {
            for part in bot::telegram_html_parts(&note) {
                sent.extend(self.send_html(chat_id, &part, Vec::new()));
            }
        }
        sent
    }

    /// The cancel reply of a request with every note its agent wrote.
    fn cancelled_reply(&self, runtime: &crate::backend_runtime::BackendRuntimeState, context: &BackendRequestContext) -> String {
        let events = runtime.request_events(context, 0).unwrap_or_default().into_iter().map(|envelope| envelope.event).collect::<Vec<_>>();
        let notes = events
            .iter()
            .filter_map(|event| match event {
                crate::backend::BackendEvent::AssistantNote { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        crate::backend::turn_interrupts::cancelled_reply(&notes)
    }

    /// Deletes messages of the chat: the agent's notes once its answer is there.
    fn delete_messages(&self, chat_id: i64, ids: &[i64]) {
        for id in ids {
            let _ = block_on(telegram::delete_message(&self.token, chat_id, *id));
        }
    }

    /// Like `send_markdown`, telling whether every part arrived.
    fn send_markdown_checked(&self, chat_id: i64, markdown: &str) -> bool {
        bot::telegram_html_parts(markdown).into_iter().all(|part| self.send_html(chat_id, &part, Vec::new()).is_some())
    }

    /// Sends the HTML parts of one text in order; the buttons go with the
    /// last, whose id and text it returns.
    fn send_parts(&self, chat_id: i64, parts: Vec<String>, buttons: Vec<(String, String)>) -> Option<(i64, String)> {
        let last = parts.len().saturating_sub(1);
        let mut sent = None;
        for (index, part) in parts.into_iter().enumerate() {
            let id = self.send_html(chat_id, &part, if index == last { buttons.clone() } else { Vec::new() });
            if index == last {
                sent = id.map(|id| (id, part));
            }
        }
        sent
    }

    /// Rewrites a sent message (HTML), which loses its buttons; as plain
    /// text when Telegram cannot parse the HTML. An unchanged text is left
    /// as it is.
    fn edit_html(&self, chat_id: i64, message_id: i64, html: &str) {
        if let Err(error) = block_on(telegram::edit_message(&self.token, chat_id, message_id, html, Vec::new(), Some("HTML"))) {
            if error.contains("parse") {
                let _ = block_on(telegram::edit_message(&self.token, chat_id, message_id, &strip_html(html), Vec::new(), None));
            }
        }
    }

    fn notify_interrupted(&self) {
        let jobs = self.interrupted.lock().map(|jobs| jobs.clone()).unwrap_or_default();
        let mut by_chat = HashMap::<i64, usize>::new();
        for job in &jobs {
            *by_chat.entry(job.chat_id).or_default() += 1;
        }
        for (chat_id, count) in by_chat {
            self.send(chat_id, &bot::interrupted_message(count));
        }
    }

    fn database_context(&self) -> LibraryDatabaseContext {
        LibraryDatabaseContext {
            library_path: self.library.path.clone(),
            android_directory_uri: self.library.android_tree_uri.clone(),
        }
    }

    fn library_config(&self) -> Value {
        crate::library_config::read_library_config(&self.app, &self.library.id).ok().flatten().unwrap_or(Value::Null)
    }

    // -----------------------------------------------------------------------
    // Polling
    // -----------------------------------------------------------------------

    fn poll(self: Arc<Self>) {
        while !self.stopped() {
            let offset = self.persisted.lock().map(|file| file.offset).unwrap_or_default();
            let updates = match block_on(telegram::get_updates(&self.token, offset)) {
                Ok(updates) => updates,
                Err(_) => {
                    std::thread::sleep(POLL_RETRY_DELAY);
                    continue;
                }
            };
            for update in updates {
                if self.stopped() {
                    break;
                }
                let seen = {
                    let Ok(mut file) = self.persisted.lock() else {
                        return;
                    };
                    file.offset = file.offset.max(update.update_id + 1);
                    let seen = file.processed.contains(&update.update_id);
                    if !seen {
                        file.processed.push_back(update.update_id);
                        while file.processed.len() > MAX_PROCESSED_UPDATES {
                            file.processed.pop_front();
                        }
                    }
                    seen
                };
                self.persist();
                if !seen {
                    self.handle_update(update);
                }
            }
        }
        // Stopping keeps the queue on disk: it becomes interrupted and is
        // only resumed with the explicit recovery command.
        if let Ok(mut queue) = self.queue.lock() {
            // Autonomous runs are dropped (the next review asks again), and
            // so are action runs (their runs end up interrupted).
            let drained = queue.drain(..).filter(|job| job.is_persisted()).map(|job| job.stored).collect::<Vec<_>>();
            if let Ok(mut interrupted) = self.interrupted.lock() {
                interrupted.extend(drained);
            }
        }
        self.queue_ready.notify_all();
        self.persist();
    }

    fn handle_update(self: &Arc<Self>, update: IncomingTelegramUpdate) {
        if update.chat_type != "private" {
            self.send(update.chat_id, "Notia solo responde en chats privados.");
            return;
        }
        let Some(library_user_id) = self.linked_user(&update) else {
            return;
        };
        if let Some(callback_id) = update.callback_query_id.as_deref() {
            let _ = block_on(telegram::answer_callback(&self.token, callback_id));
        }
        if let Some(data) = update.callback_data.as_deref() {
            // The approval of an AI task: only the Owner answers it.
            if data.starts_with(crate::ai_task_runner::TELEGRAM_CALLBACK_PREFIX) {
                if library_user_id == OWNER {
                    crate::ai_task_runner::answer_from_telegram(data);
                }
                return;
            }
            self.handle_callback(update.chat_id, data);
            return;
        }
        let mut text = update.text.as_deref().map(str::trim).unwrap_or_default().to_string();
        // A reply («Responder») to the message of an AI task's question or
        // approval answers the task instead of starting a request.
        if let (OWNER, Some(reply_to)) = (library_user_id.as_str(), update.reply_to_message_id) {
            if crate::ai_task_runner::answer_reply_from_telegram(update.chat_id, reply_to, &text) {
                self.send(update.chat_id, "Le pasé tu respuesta a la tarea.");
                return;
            }
        }
        if text.is_empty() {
            if let Some(audio) = update.audio.as_ref() {
                match transcribe(&self.app, &self.token, audio) {
                    Ok(transcription) => {
                        text = format!(
                            "{}\n\n[Origen: audio de Telegram fileId={}; conservar esta referencia si se crea una operación financiera.]",
                            transcription.trim(),
                            audio_file_id(audio)
                        );
                        let acknowledgement = transcription.chars().take(3_000).collect::<String>();
                        self.send_html(
                            update.chat_id,
                            &format!("Audio recibido: <b>{}</b>", crate::backend::escape_telegram_html(&acknowledgement)),
                            Vec::new(),
                        );
                    }
                    Err(message) => {
                        self.send(update.chat_id, &message);
                        return;
                    }
                }
            }
        }
        if update.document.as_ref().is_some_and(|document| telegram::document_kind(document).is_none()) {
            self.send(update.chat_id, "Por ahora recibo fotos, imágenes JPG o PNG y documentos PDF.");
            return;
        }
        let attachment = update
            .photo
            .clone()
            .map(JobAttachment::Photo)
            .or_else(|| update.document.clone().map(JobAttachment::Document));
        if text.is_empty() && attachment.is_none() {
            return;
        }
        // The Owner comes first: a run Notia started by itself gives way.
        self.stop_autonomous_run(update.chat_id);
        let (chat_id, telegram_user_id) = (update.chat_id, update.user.id);
        // Stopping what is still being written needs no request.
        if crate::backend::turn_interrupts::is_cancel_command(&text) && self.drop_batch(chat_id) {
            self.send(chat_id, "Listo, descarté lo que me estabas escribiendo.");
            return;
        }
        // A new request waits for the rest of what the person writes; a
        // command, an answer to the chat's question and a message for its
        // running request go on at once, after what was waiting.
        let batched = !text.starts_with('/') && self.waits_for_more(chat_id);
        if !batched {
            self.flush_batch(chat_id);
        }
        match (attachment, update.media_group_id) {
            (Some(attachment), Some(group)) => {
                self.collect_album_part(chat_id, group, telegram_user_id, library_user_id, text, attachment, batched)
            }
            (Some(attachment), None) if batched => self.collect_batch_part(chat_id, telegram_user_id, library_user_id, text, vec![attachment]),
            (Some(attachment), None) => self.enqueue(chat_id, telegram_user_id, library_user_id, text, vec![attachment], true),
            (None, _) if batched => self.collect_batch_part(chat_id, telegram_user_id, library_user_id, text, Vec::new()),
            (None, _) => self.dispatch_text(chat_id, telegram_user_id, library_user_id, text),
        }
    }

    /// Whether a message of `chat_id` starts or joins a new request: the
    /// chat has no question waiting for its answer and no request of its
    /// own running.
    fn waits_for_more(&self, chat_id: i64) -> bool {
        let asking = self.prompt.lock().map(|prompt| prompt.as_ref().is_some_and(|(prompt_chat, ..)| *prompt_chat == chat_id)).unwrap_or(false);
        !asking && self.running_request(chat_id).is_none()
    }

    /// Adds a message to the batch of `chat_id`, which goes on
    /// `TELEGRAM_WAIT` after its last message.
    fn collect_batch_part(self: &Arc<Self>, chat_id: i64, telegram_user_id: i64, library_user_id: String, text: String, attachments: Vec<JobAttachment>) {
        let wait = crate::backend::reply_wait::TELEGRAM_WAIT;
        let opened = match self.batches.lock() {
            Ok(mut batches) => add_to_batch(&mut batches, chat_id, telegram_user_id, library_user_id, text, attachments, wait),
            Err(_) => {
                if attachments.is_empty() {
                    self.dispatch_text(chat_id, telegram_user_id, library_user_id, text);
                } else {
                    self.enqueue(chat_id, telegram_user_id, library_user_id, text, attachments, true);
                }
                return;
            }
        };
        self.persist();
        if opened {
            self.wait_for_batch(chat_id);
        }
    }

    /// Sends the batch of `chat_id` on once its time came, unless another
    /// message sent it before. A worker that stops keeps it on disk, as an
    /// interrupted request.
    fn wait_for_batch(self: &Arc<Self>, chat_id: i64) {
        let worker = Arc::clone(self);
        let spawned = std::thread::Builder::new().name("notia-telegram-batch".into()).spawn(move || loop {
            std::thread::sleep(BATCH_POLL_INTERVAL);
            if worker.stopped() {
                return;
            }
            let complete = {
                let Ok(mut batches) = worker.batches.lock() else {
                    return;
                };
                match batches.get(&chat_id) {
                    Some(batch) if batch.ready_at <= Instant::now() => batches.remove(&chat_id),
                    Some(_) => continue,
                    None => return,
                }
            };
            if let Some(batch) = complete {
                worker.dispatch_batch(chat_id, batch);
            }
            return;
        });
        if spawned.is_err() {
            self.flush_batch(chat_id);
        }
    }

    /// Sends on at once the batch of `chat_id`.
    fn flush_batch(self: &Arc<Self>, chat_id: i64) {
        let batch = self.batches.lock().ok().and_then(|mut batches| batches.remove(&chat_id));
        if let Some(batch) = batch {
            self.dispatch_batch(chat_id, batch);
        }
    }

    /// Drops the batch of `chat_id`; whether there was one.
    fn drop_batch(&self, chat_id: i64) -> bool {
        let dropped = self.batches.lock().ok().and_then(|mut batches| batches.remove(&chat_id)).is_some();
        if dropped {
            self.persist();
        }
        dropped
    }

    fn dispatch_batch(self: &Arc<Self>, chat_id: i64, batch: PendingBatch) {
        let text = batch.text();
        if batch.attachments.is_empty() {
            self.dispatch_text(chat_id, batch.telegram_user_id, batch.library_user_id, text);
        } else {
            self.enqueue(chat_id, batch.telegram_user_id, batch.library_user_id, text, batch.attachments, true);
        }
        self.persist();
    }

    /// Sends on a whole typed or dictated text: the recovery command, the
    /// answer to the pending confirmation or question, a message for the
    /// chat's running request, or a new request.
    fn dispatch_text(self: &Arc<Self>, chat_id: i64, telegram_user_id: i64, library_user_id: String, text: String) {
        if text.to_lowercase() == bot::RECOVERY_COMMAND {
            self.recover(chat_id);
            return;
        }
        if self.answer_prompt(chat_id, &text) {
            return;
        }
        // A message of the chat whose request runs may ask to stop it; the
        // decision runs beside the request, so the polling goes on.
        if self.running_request(chat_id).is_some() {
            let worker = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("notia-telegram-interrupt".into())
                .spawn(move || worker.interrupt(chat_id, telegram_user_id, library_user_id, text));
            return;
        }
        self.enqueue(chat_id, telegram_user_id, library_user_id, text, Vec::new(), true);
    }

    /// Adds a part of an album; the first part waits until no other part
    /// arrived for `ALBUM_WINDOW` and sends the whole album on as one
    /// message, with the caption any of its parts carried: into the chat's
    /// batch when `batched`, else queued at once.
    #[allow(clippy::too_many_arguments)]
    fn collect_album_part(
        self: &Arc<Self>,
        chat_id: i64,
        group: String,
        telegram_user_id: i64,
        library_user_id: String,
        text: String,
        attachment: JobAttachment,
        batched: bool,
    ) {
        let key = (chat_id, group);
        let first = {
            let Ok(mut albums) = self.albums.lock() else {
                return;
            };
            match albums.get_mut(&key) {
                Some(album) => {
                    if album.attachments.len() < MAX_ALBUM_PARTS {
                        album.attachments.push(attachment);
                    }
                    if album.text.is_empty() {
                        album.text = text;
                    }
                    album.last_part = Instant::now();
                    false
                }
                None => {
                    albums.insert(
                        key.clone(),
                        PendingAlbum { telegram_user_id, library_user_id, text, attachments: vec![attachment], last_part: Instant::now(), batched },
                    );
                    true
                }
            }
        };
        if !first {
            return;
        }
        let worker = Arc::clone(self);
        let _ = std::thread::Builder::new().name("notia-telegram-album".into()).spawn(move || loop {
            std::thread::sleep(PART_POLL_INTERVAL);
            let complete = {
                let Ok(mut albums) = worker.albums.lock() else {
                    return;
                };
                match albums.get(&key) {
                    Some(album) if album.last_part.elapsed() >= ALBUM_WINDOW => albums.remove(&key),
                    Some(_) => continue,
                    None => return,
                }
            };
            if let Some(album) = complete {
                if album.batched {
                    worker.collect_batch_part(chat_id, album.telegram_user_id, album.library_user_id, album.text, album.attachments);
                } else {
                    worker.enqueue(chat_id, album.telegram_user_id, album.library_user_id, album.text, album.attachments, true);
                }
            }
            return;
        });
    }

    /// Text and context of the request that runs for `chat_id` because the
    /// person asked for it (an autonomous run never decides about messages).
    fn running_request(&self, chat_id: i64) -> Option<(String, BackendRequestContext)> {
        self.current
            .lock()
            .ok()?
            .as_ref()
            .filter(|run| run.chat_id == chat_id && !run.autonomous)
            .map(|run| (run.text.clone(), run.context.clone()))
    }

    /// Drops the queued autonomous run of `chat_id` and cancels, without
    /// telling the chat, the one that runs, so the message just received
    /// goes first.
    fn stop_autonomous_run(self: &Arc<Self>, chat_id: i64) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.retain(|job| job.autonomous.is_none() || job.stored.chat_id != chat_id);
        }
        let request_id = self.current.lock().ok().and_then(|current| {
            current.as_ref().filter(|run| run.chat_id == chat_id && run.autonomous).map(|run| run.context.request_id.clone())
        });
        let Some(request_id) = request_id else {
            return;
        };
        let worker = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("notia-telegram-autonomy-stop".into())
            .spawn(move || worker.cancel_current(chat_id, &request_id));
    }

    /// Queues an autonomous run in the Owner's chat, unless one is already
    /// queued or running or the Owner has not linked Telegram.
    fn enqueue_autonomous(&self, kind: AutonomousKind, trigger: String) -> AutonomousEnqueue {
        let running = self.current.lock().map(|current| current.as_ref().is_some_and(|run| run.autonomous)).unwrap_or(true);
        let queued = self.queue.lock().map(|queue| queue.iter().any(|job| job.autonomous.is_some())).unwrap_or(true);
        if running || queued {
            return AutonomousEnqueue::Busy;
        }
        let Ok(Some((telegram_user_id, chat_id))) = crate::library_users::owner_telegram_link(&self.app, &self.database_context()) else {
            return AutonomousEnqueue::Unavailable;
        };
        // Only the autonomy thread queues these runs, so none arrived meanwhile.
        let Ok(mut queue) = self.queue.lock() else {
            return AutonomousEnqueue::Unavailable;
        };
        queue.push_back(Job {
            stored: StoredJob {
                request_id: short_id(),
                chat_id,
                telegram_user_id,
                library_user_id: OWNER.to_string(),
                attachments: Vec::new(),
                attachment: None,
            },
            text: trigger,
            autonomous: Some(kind),
            action: None,
            webhook: None,
        });
        drop(queue);
        self.queue_ready.notify_all();
        AutonomousEnqueue::Queued
    }

    /// Queues the run of an AI action or a webhook call in the Owner's
    /// chat, after what is already waiting; the Owner must have linked
    /// Telegram.
    fn enqueue_owner_run(&self, text: String, action: Option<ActionJob>, webhook: Option<WebhookJob>) -> ActionEnqueue {
        let Ok(Some((telegram_user_id, chat_id))) = crate::library_users::owner_telegram_link(&self.app, &self.database_context()) else {
            return ActionEnqueue::Unavailable;
        };
        let Ok(mut queue) = self.queue.lock() else {
            return ActionEnqueue::Unavailable;
        };
        queue.push_back(Job {
            stored: StoredJob {
                request_id: short_id(),
                chat_id,
                telegram_user_id,
                library_user_id: OWNER.to_string(),
                attachments: Vec::new(),
                attachment: None,
            },
            text,
            autonomous: None,
            action,
            webhook,
        });
        drop(queue);
        self.queue_ready.notify_all();
        ActionEnqueue::Queued
    }

    fn cancel_requested(&self, chat_id: i64) -> bool {
        self.current
            .lock()
            .ok()
            .and_then(|current| current.as_ref().filter(|run| run.chat_id == chat_id).map(|run| run.cancelled.load(Ordering::SeqCst)))
            .unwrap_or(false)
    }

    /// Decides whether a message sent during the chat's request stops it,
    /// stops it and runs next, or waits in the queue.
    fn interrupt(self: &Arc<Self>, chat_id: i64, telegram_user_id: i64, library_user_id: String, text: String) {
        let Some((running, context)) = self.running_request(chat_id) else {
            // The request ended meanwhile: the message is a new one.
            self.enqueue(chat_id, telegram_user_id, library_user_id, text, Vec::new(), true);
            return;
        };
        let decision = crate::backend_runtime::classify_interrupt(&self.app, &context, &running, &text);
        match (decision.cancels(), decision.queues()) {
            (true, true) => {
                self.send_html(chat_id, &format!("{} Después sigo con tu nuevo pedido.", bot::CANCELLING_MESSAGE), Vec::new());
                self.enqueue(chat_id, telegram_user_id, library_user_id, text, Vec::new(), false);
                self.cancel_current(chat_id, &context.request_id);
            }
            (true, false) => {
                self.send_html(chat_id, bot::CANCELLING_MESSAGE, Vec::new());
                self.cancel_current(chat_id, &context.request_id);
            }
            _ => self.enqueue(chat_id, telegram_user_id, library_user_id, text, Vec::new(), true),
        }
    }

    /// Stops the chat's request `request_id`: the question or confirmation
    /// it waits on is dropped, or its run is cancelled. A cancel that
    /// arrives before the run registered its control is tried again while
    /// that request is still the current one.
    fn cancel_current(&self, chat_id: i64, request_id: &str) {
        let target = self.current.lock().ok().and_then(|current| {
            current.as_ref().filter(|run| run.chat_id == chat_id && run.context.request_id == request_id).map(|run| {
                run.cancelled.store(true, Ordering::SeqCst);
                (run.context.clone(), run.idempotency_key.clone())
            })
        });
        let Some((context, idempotency_key)) = target else {
            return;
        };
        let dropped_prompt = self
            .prompt
            .lock()
            .map(|mut prompt| {
                let waiting = prompt.as_ref().is_some_and(|(prompt_chat, ..)| *prompt_chat == chat_id);
                if waiting {
                    *prompt = None;
                }
                waiting
            })
            .unwrap_or(false);
        if dropped_prompt {
            return;
        }
        let runtime = self.app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
        for _ in 0..CANCEL_ATTEMPTS {
            let still_current = self
                .current
                .lock()
                .map(|current| current.as_ref().is_some_and(|run| run.context.request_id == request_id))
                .unwrap_or(false);
            if !still_current {
                return;
            }
            let cancelled = {
                let registry = self.app.state::<crate::library_registry::LibraryBindingRegistry>();
                crate::backend_runtime::execute_backend_request(
                    &self.app,
                    cancel_envelope(&context, &idempotency_key, None),
                    &runtime,
                    registry.inner(),
                )
            };
            if cancelled.is_ok_and(|envelope| !matches!(envelope.response, BackendResponse::Error { .. })) {
                return;
            }
            std::thread::sleep(CANCEL_RETRY_DELAY);
        }
    }

    fn handle_callback(&self, chat_id: i64, data: &str) {
        let Ok(prompt) = self.prompt.lock() else {
            return;
        };
        let Some((prompt_chat, kind, sender)) = prompt.as_ref() else {
            return;
        };
        if *prompt_chat != chat_id {
            return;
        }
        match kind {
            Prompt::Confirmation { id } => {
                let mut parts = data.split(':');
                if parts.next() == Some("confirm") && parts.next() == Some(id.as_str()) {
                    if let Some(reply) = parts.next().and_then(bot::ConfirmationReply::from_id) {
                        let _ = sender.send(Reply::Decision(reply, true));
                    }
                }
            }
            Prompt::Proposal => {}
            Prompt::Clarification { choices } => {
                if let Some(index) = data.strip_prefix("choice:").and_then(|value| value.parse::<usize>().ok()) {
                    if let Some(choice) = choices.get(index) {
                        let _ = sender.send(Reply::Text(choice.clone(), Some(index), true));
                    }
                }
            }
        }
    }

    /// Delivers a typed answer to the pending confirmation or question.
    fn answer_prompt(&self, chat_id: i64, text: &str) -> bool {
        let Ok(prompt) = self.prompt.lock() else {
            return false;
        };
        let Some((prompt_chat, kind, sender)) = prompt.as_ref() else {
            return false;
        };
        if *prompt_chat != chat_id {
            return false;
        }
        match kind {
            Prompt::Confirmation { .. } => match bot::parse_confirmation_decision(text) {
                Some(decision) => sender.send(Reply::Decision(decision, false)).is_ok(),
                None => false,
            },
            Prompt::Clarification { choices } => {
                let (answer, index) = bot::resolve_choice_reply(text, choices);
                sender.send(Reply::Text(answer, index, false)).is_ok()
            }
            Prompt::Proposal => sender.send(Reply::Text(text.trim().to_string(), None, false)).is_ok(),
        }
    }

    fn recover(&self, chat_id: i64) {
        let recovered = self
            .interrupted
            .lock()
            .map(|mut items| {
                let (mine, others): (Vec<_>, Vec<_>) = items.drain(..).partition(|job| job.chat_id == chat_id);
                *items = others;
                mine
            })
            .unwrap_or_default();
        if recovered.is_empty() {
            self.send(chat_id, "No hay solicitudes interrumpidas para reanudar.");
            return;
        }
        let (recoverable, resend): (Vec<_>, Vec<_>) = recovered.into_iter().partition(|job| !job.attachments.is_empty());
        let count = recoverable.len();
        if let Ok(mut queue) = self.queue.lock() {
            queue.extend(recoverable.into_iter().map(|stored| Job { stored, text: String::new(), autonomous: None, action: None, webhook: None }));
        }
        self.persist();
        self.queue_ready.notify_all();
        let mut message = if count > 0 {
            format!("{count} solicitud(es) con documento marcada(s) para reanudar.")
        } else {
            "No hay solicitudes con contenido recuperable.".to_string()
        };
        if !resend.is_empty() {
            message.push_str(&format!(
                " {} solicitud(es) de texto requieren que las reenvíes: no guardo el texto original para proteger tu privacidad.",
                resend.len()
            ));
        }
        self.send(chat_id, &message);
    }

    fn enqueue(
        &self,
        chat_id: i64,
        telegram_user_id: i64,
        library_user_id: String,
        text: String,
        attachments: Vec<JobAttachment>,
        announce: bool,
    ) {
        let document = !attachments.is_empty();
        let ahead = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            let pending = queue.iter().filter(|job| job.is_persisted()).count();
            if pending >= bot::MAX_PENDING_REQUESTS {
                drop(queue);
                self.send(chat_id, "No puedo aceptar más de 10 solicitudes pendientes. Esperá a que termine alguna e intentá nuevamente.");
                return;
            }
            let ahead = pending + usize::from(self.active.lock().map(|active| active.is_some()).unwrap_or(false));
            queue.push_back(Job {
                stored: StoredJob {
                    request_id: short_id(),
                    chat_id,
                    telegram_user_id,
                    library_user_id,
                    attachments,
                    attachment: None,
                },
                text,
                autonomous: None,
                action: None,
                webhook: None,
            });
            ahead
        };
        self.persist();
        self.queue_ready.notify_all();
        if announce && ahead > 0 {
            self.send_html(chat_id, &bot::queued_message(ahead, document), Vec::new());
        }
    }

    // -----------------------------------------------------------------------
    // Linking
    // -----------------------------------------------------------------------

    /// Library user linked to this Telegram account, or `None` while the
    /// account is being linked (the flow answers the chat itself).
    fn linked_user(&self, update: &IncomingTelegramUpdate) -> Option<String> {
        let context = self.database_context();
        let resolved = crate::library_users::resolve_library_telegram_user(
            self.app.clone(),
            crate::library_users::ResolveTelegramUserPayload {
                context: context.clone(),
                telegram_user_id: update.user.id,
                telegram_chat_id: update.chat_id,
            },
        );
        match resolved {
            Ok(Some(user)) => return Some(user.id),
            Ok(None) => {}
            Err(_) => {
                self.send(update.chat_id, "No se pudo verificar el enlace. Intentá nuevamente en unos segundos.");
                return None;
            }
        }
        self.link_step(update, context);
        None
    }

    fn link_step(&self, update: &IncomingTelegramUpdate, context: LibraryDatabaseContext) {
        let chat_id = update.chat_id;
        let now = Instant::now();
        let text = update.text.as_deref().map(str::trim).unwrap_or_default().to_string();
        let command = text.to_lowercase();
        let Ok(mut links) = self.links.lock() else {
            return;
        };
        if links.get(&chat_id).and_then(|flow| flow.blocked_until).is_some_and(|until| until > now)
            && !matches!(command.as_str(), "/start" | "/cancelar" | "/cancel")
        {
            drop(links);
            self.send(chat_id, "Demasiados intentos. Esperá unos segundos antes de volver a intentar o escribí /start para reiniciar el enlace.");
            return;
        }
        if command == "/start" {
            links.insert(chat_id, LinkFlow { step: LinkStep::Username, user_id: None, expires_at: now + LINK_TIMEOUT, attempts: 0, blocked_until: None });
            drop(links);
            self.send(chat_id, "Para vincular Telegram, escribí tu nombre de usuario de Notia. Escribí /cancelar para detener el enlace.");
            return;
        }
        if matches!(command.as_str(), "/cancelar" | "/cancel") {
            links.remove(&chat_id);
            drop(links);
            self.send(chat_id, "Enlace cancelado. Escribí /start cuando quieras intentarlo nuevamente.");
            return;
        }
        let Some(mut flow) = links.get(&chat_id).cloned().filter(|flow| flow.expires_at > now) else {
            links.remove(&chat_id);
            drop(links);
            self.send(chat_id, "No tenés un usuario de Notia vinculado. Escribí /start para iniciar sesión y vincular este chat.");
            return;
        };
        if text.is_empty() || update.audio.is_some() || update.photo.is_some() || update.document.is_some() || update.callback_query_id.is_some() {
            drop(links);
            self.send(chat_id, "Durante el enlace solo se aceptan respuestas de texto. Escribí /cancelar para detenerlo.");
            return;
        }
        let fail = |flow: &mut LinkFlow| {
            flow.attempts += 1;
            if flow.attempts >= LINK_MAX_ATTEMPTS {
                flow.blocked_until = Some(now + LINK_COOLDOWN);
            }
        };
        let reply = match flow.step.clone() {
            LinkStep::Username => {
                match crate::library_users::find_library_user(
                    self.app.clone(),
                    crate::library_users::FindLibraryUserPayload { context, name: text },
                ) {
                    Ok(Some(user)) => {
                        flow.step = if user.password_configured { LinkStep::ExistingPassword } else { LinkStep::NewPassword };
                        flow.user_id = Some(user.id);
                        if user.password_configured {
                            "Escribí la contraseña de tu usuario de Notia."
                        } else {
                            "Este usuario todavía no tiene contraseña. Escribí una nueva de 8 a 256 caracteres."
                        }
                    }
                    Ok(None) => {
                        fail(&mut flow);
                        "No se pudo completar el enlace con esos datos. Revisá el nombre e intentá nuevamente."
                    }
                    Err(_) => "No se pudo completar el enlace. Intentá nuevamente.",
                }
            }
            LinkStep::NewPassword => {
                if !(8..=256).contains(&text.chars().count()) {
                    "La contraseña debe tener entre 8 y 256 caracteres. Intentá nuevamente."
                } else {
                    flow.step = LinkStep::ConfirmPassword(text);
                    "Repetí la nueva contraseña para confirmarla."
                }
            }
            LinkStep::ConfirmPassword(password) if password != text => {
                flow.step = LinkStep::NewPassword;
                fail(&mut flow);
                "Las contraseñas no coinciden. Escribí una nueva contraseña para intentarlo otra vez."
            }
            LinkStep::ConfirmPassword(_) | LinkStep::ExistingPassword => {
                let linked = crate::library_users::link_library_user_telegram(
                    self.app.clone(),
                    crate::library_users::LinkTelegramPayload {
                        context,
                        user_id: flow.user_id.clone().unwrap_or_default(),
                        telegram_user_id: update.user.id,
                        telegram_chat_id: chat_id,
                        password: text,
                    },
                );
                if linked.is_ok() {
                    links.remove(&chat_id);
                    drop(links);
                    self.send(chat_id, "Telegram quedó vinculado. Ya podés enviar consultas.");
                    return;
                }
                fail(&mut flow);
                if flow.blocked_until.is_some() {
                    "Se alcanzó el límite de intentos. Escribí /start más tarde para reintentar."
                } else {
                    "No se pudo verificar la contraseña. Intentá nuevamente o escribí /cancelar."
                }
            }
        };
        links.insert(chat_id, flow);
        drop(links);
        self.send(chat_id, reply);
    }

    // -----------------------------------------------------------------------
    // Execution
    // -----------------------------------------------------------------------

    fn run_queue(self: Arc<Self>) {
        while !self.stopped() {
            let job = {
                let Ok(queue) = self.queue.lock() else {
                    return;
                };
                let Ok((mut queue, _)) = self.queue_ready.wait_timeout(queue, Duration::from_secs(1)) else {
                    return;
                };
                queue.pop_front()
            };
            let Some(job) = job else {
                continue;
            };
            // Runs Notia started by itself are never stored nor resumed.
            if let Some(action) = job.action.clone() {
                self.run_action(&job, &action);
                if let Ok(mut current) = self.current.lock() {
                    *current = None;
                }
                continue;
            }
            if let Some(webhook) = job.webhook.clone() {
                self.run_webhook(&job, &webhook);
                if let Ok(mut current) = self.current.lock() {
                    *current = None;
                }
                continue;
            }
            if let Some(kind) = job.autonomous {
                self.run_autonomous(&job, kind);
                if let Ok(mut current) = self.current.lock() {
                    *current = None;
                }
                continue;
            }
            if let Ok(mut active) = self.active.lock() {
                *active = Some(job.stored.clone());
            }
            self.persist();
            let chat_id = job.stored.chat_id;
            let result = self.run_job(&job);
            if let Ok(mut current) = self.current.lock() {
                *current = None;
            }
            match result {
                // A cancel may carry the agent's findings, written in Markdown.
                Err(message) if message.starts_with(bot::CANCELLED_MESSAGE) => {
                    self.send_markdown_checked(chat_id, &message);
                }
                Err(message) => {
                    self.send(chat_id, &message);
                }
                Ok(()) => {}
            }
            if let Ok(mut active) = self.active.lock() {
                *active = None;
            }
            self.persist();
        }
    }

    /// Text and files of the request, downloading its attachments. Each
    /// file adds an origin line with its evidence reference; photos and
    /// images go as images and a PDF as its extracted text. A file that
    /// cannot be read is named in the request while the others go on.
    fn prepare_input(&self, job: &Job) -> Result<(String, Vec<MessageAttachment>), String> {
        let text = job.text.trim().to_string();
        let files = &job.stored.attachments;
        if files.is_empty() {
            if text.is_empty() {
                return Err("La solicitud perdió su texto al reiniciar; reenviala.".to_string());
            }
            return Ok((text, Vec::new()));
        }
        let mut origins = Vec::new();
        let mut attachments = Vec::new();
        let mut unread = Vec::new();
        for (index, file) in files.iter().enumerate() {
            match self.read_attachment(index + 1, file) {
                Ok((origin, attachment)) => {
                    origins.push(origin);
                    attachments.push(attachment);
                }
                Err(reason) => unread.push(reason),
            }
        }
        if attachments.is_empty() {
            return Err(unread.into_iter().next().unwrap_or_else(|| "No se pudo leer el archivo.".to_string()));
        }
        let request = if text.is_empty() { bot::DOCUMENT_PROMPT.to_string() } else { text };
        let mut prompt = format!("{request}\n\n{}", origins.join("\n"));
        if !unread.is_empty() {
            prompt.push_str(&format!("\n[No se pudieron leer: {}]", unread.join(" ")));
        }
        Ok((prompt, attachments))
    }

    /// Downloads one file of the request: its origin line and its attachment.
    fn read_attachment(&self, number: usize, file: &JobAttachment) -> Result<(String, MessageAttachment), String> {
        use crate::backend::chat_attachments::{MessageAttachmentKind, MAX_TEXT_CHARS};
        let image = |name: String, media_type: &str, bytes: Vec<u8>| MessageAttachment {
            name,
            media_type: media_type.to_string(),
            kind: MessageAttachmentKind::Image,
            pages: vec![base64::engine::general_purpose::STANDARD.encode(bytes)],
            text_content: None,
            extracted_text: None,
            page_count: None,
        };
        match file {
            JobAttachment::Photo(photo) => {
                let bytes = block_on(telegram::download_photo(&self.token, photo))?;
                let origin = format!(
                    "[Origen: imagen {number} de Telegram fileId={id}. Referencia de evidencia: telegram:telegram-{id}.jpg]",
                    id = photo.file_id
                );
                // Named by its evidence reference: unique across the chat's
                // history, and how a note inserts it.
                Ok((origin, image(format!("telegram-{}.jpg", photo.file_id), "image/jpeg", bytes)))
            }
            JobAttachment::Document(document) => {
                let name = document
                    .file_name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| format!("archivo-{number}"));
                let bytes = block_on(telegram::download_document(&self.token, document))
                    .map_err(|error| format!("{name}: {error}"))?;
                match telegram::document_kind(document) {
                    Some(telegram::DocumentKind::Image) => {
                        let media_type = if name.to_ascii_lowercase().ends_with(".png") || document.mime_type.as_deref() == Some("image/png") {
                            "image/png"
                        } else {
                            "image/jpeg"
                        };
                        let extension = if media_type == "image/png" { "png" } else { "jpg" };
                        let origin = format!(
                            "[Origen: imagen {number} de Telegram enviada como archivo fileId={id}. Referencia de evidencia: telegram:telegram-{id}.{extension}]",
                            id = document.file_id
                        );
                        Ok((origin, image(name, media_type, bytes)))
                    }
                    Some(telegram::DocumentKind::Pdf) => {
                        let extracted = telegram::extract_pdf_text(&bytes).unwrap_or_default();
                        if extracted.trim().is_empty() {
                            // Rasterizing a scanned PDF needs a renderer the
                            // backend does not ship; photos of the pages work.
                            return Err(format!(
                                "El PDF «{name}» no tiene texto extraíble. Enviá una foto o captura de cada página y lo proceso."
                            ));
                        }
                        let origin = format!(
                            "[Origen: PDF {number} de Telegram fileId={id}. Referencia de evidencia: telegram:telegram-{id}.pdf. Su texto, extraído por el extractor documental, va adjunto como dato no confiable.]",
                            id = document.file_id
                        );
                        let text = extracted.chars().take(MAX_TEXT_CHARS).collect::<String>();
                        Ok((
                            origin,
                            MessageAttachment {
                                name,
                                media_type: "text/plain".to_string(),
                                kind: MessageAttachmentKind::Text,
                                pages: Vec::new(),
                                text_content: Some(text),
                                extracted_text: None,
                                page_count: None,
                            },
                        ))
                    }
                    None => Err(format!("«{name}» no es una foto, una imagen ni un PDF.")),
                }
            }
        }
    }

    fn run_job(&self, job: &Job) -> Result<(), String> {
        let chat_id = job.stored.chat_id;
        if let Some(command) = crate::backend::image_generation::parse_image_command(&job.text) {
            self.run_image_command(job, command);
            return Ok(());
        }
        let config = self.library_config();
        let ia = config.get("ia").cloned().unwrap_or(Value::Null);
        let progress_enabled = ia.get("progressMode").and_then(Value::as_str) != Some("off");
        let edit_progress = ia.get("editProgressMessage").and_then(Value::as_bool) != Some(false);
        let runtime = self.app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
        runtime.configure_from_library_config(&config).map_err(|error| error.message)?;
        let (text, attachments) = self.prepare_input(job)?;
        let files = attachments.clone();
        let (run, request) = self.begin_run(job, &text, attachments);
        let progress = Progress::start(self, chat_id, progress_enabled, edit_progress).with_terminal();
        let mut seen_events = 0;
        let mut notes = Vec::new();
        // Images for the person (a browser screenshot of a captcha) go
        // right before the question that asks about them.
        let mut shown = Vec::new();
        let outcome = self.drive(&runtime, &run, request, &progress, |interaction, operation| {
            // What the agent wrote in the round it asks is the context of
            // the question: it goes whole before it.
            notes.extend(self.deliver_notes(chat_id, &runtime, &run.context, &mut seen_events));
            shown.extend(self.send_generated_images(chat_id, &run.context.request_id));
            match self.ask(chat_id, interaction, operation) {
                Some(answer) => {
                    match answer.message {
                        // Answered with a button: the progress goes on in
                        // the question's message.
                        Some((id, text)) => progress.adopt(self, id, text, answer.acknowledgement.as_deref()),
                        None => {
                            if let Some(text) = answer.acknowledgement {
                                progress.restart(self, &text);
                            }
                        }
                    }
                    Ok(answer.decision)
                }
                None if run.cancelled.load(Ordering::SeqCst) => {
                    self.cancel_operation(&run.context, &run.idempotency_key, operation.clone());
                    Err(bot::CANCELLED_MESSAGE.to_string())
                }
                None => Err("Operación cancelada.".to_string()),
            }
        });
        let stopped = run.cancelled.load(Ordering::SeqCst);
        // The answer replaces the progress message (a question's message
        // keeps its text and answer); a cancel or a failure stays written
        // on it.
        match (&outcome, stopped) {
            (Ok(_), _) => progress.dismiss(self),
            (Err(_), true) => progress.finish(self, "<b>Solicitud cancelada</b>"),
            (Err(_), false) => progress.finish(self, "<b>No pude completar la operación</b>"),
        }
        if stopped && outcome.is_err() {
            // The next request of the chat knows what was cancelled and what
            // the agent had found, so «seguí» picks it up again.
            let reply = self.cancelled_reply(&runtime, &run.context);
            self.remember_with_files(chat_id, text, files, reply.clone());
            return Err(reply);
        }
        // A failure says whether anything was done before it, so the person
        // knows if repeating the request is safe.
        let outcome = outcome.map_err(|error| format!("{error}\n\n{}", failure_progress(&runtime, &run.context)));
        // A failed request stays in the chat's history too, so «seguí» or
        // «¿qué pasó?» has its context.
        if let Err(error) = &outcome {
            self.remember_with_files(chat_id, text.clone(), files.clone(), format!("No pude terminar: {error}"));
        }
        let response = outcome?;
        if stopped {
            self.send(chat_id, bot::TOO_LATE_TO_CANCEL_MESSAGE);
        }
        // The images the agent made go right before its text, and the
        // answer replaces the notes it wrote on the way.
        let mut drawn = shown;
        drawn.extend(self.send_generated_images(chat_id, &run.context.request_id));
        if self.send_markdown_checked(chat_id, &response.response.markdown) {
            self.delete_messages(chat_id, &notes);
        }
        self.remember_turn(chat_id, text, files, response.response.markdown.clone(), drawn);
        if response.changed {
            let _ = self.app.emit(LIBRARY_CHANGED_EVENT, &self.library.id);
        }
        Ok(())
    }

    /// Runs a `/image` message of the Owner: the text goes to the image
    /// model as written, without the agent, and the image comes back as a
    /// photo (an edit of the first image attached, or a new one). It runs as
    /// the chat's current request, so /cancelar stops it; nothing is saved
    /// in the library.
    fn run_image_command(&self, job: &Job, command: Result<crate::backend::image_generation::ImageCommand, crate::backend::BackendError>) {
        use crate::backend::chat_attachments::MessageAttachmentKind;
        let chat_id = job.stored.chat_id;
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                self.send(chat_id, &error.message);
                return;
            }
        };
        if job.stored.library_user_id != OWNER {
            self.send(chat_id, "Las imágenes las genera solo el Owner de la biblioteca.");
            return;
        }
        let mut files = Vec::new();
        for (index, file) in job.stored.attachments.iter().enumerate() {
            match self.read_attachment(index + 1, file) {
                Ok((_, attachment)) => files.push(attachment),
                Err(reason) => {
                    self.send(chat_id, &reason);
                }
            }
        }
        let reference = files
            .iter()
            .find(|file| file.kind == MessageAttachmentKind::Image)
            .and_then(|file| file.pages.first().map(|page| (file.name.clone(), page.clone())));
        let notice = self.send(chat_id, if reference.is_some() { "Editando la imagen…" } else { "Generando la imagen…" });
        let (run, _) = self.begin_run(job, &job.text, Vec::new());
        let control = crate::backend::RequestControl::new(None);
        let done = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&run.cancelled);
        crate::image_generation::cancel_when(&control, Arc::clone(&done), move || cancelled.load(Ordering::SeqCst));
        let result = crate::image_generation::run_command(&self.app, &self.library.id, &control, &run.context.request_id, &command, reference);
        done.store(true, Ordering::SeqCst);
        if let Some(id) = notice {
            let _ = block_on(telegram::delete_message(&self.token, chat_id, id));
        }
        let (answer, drawn) = match result {
            Ok(text) => {
                let drawn = self.send_generated_images(chat_id, &run.context.request_id);
                if !drawn.is_empty() {
                    self.send(chat_id, &text);
                }
                (text, drawn)
            }
            Err(error) => {
                self.send(chat_id, &error.message);
                (error.message, Vec::new())
            }
        };
        self.remember_turn(chat_id, job.text.clone(), files, answer, drawn);
    }

    /// Runs a request Notia queued by itself. Nothing reaches the chat but
    /// the agent's message, and only when it is not silence; the message
    /// then joins the chat's history, so a reply has its context, and
    /// becomes a thought, so the next runs know it was said.
    fn run_autonomous(&self, job: &Job, kind: AutonomousKind) {
        let chat_id = job.stored.chat_id;
        let runtime = self.app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
        if let Err(error) = runtime.configure_from_library_config(&self.library_config()) {
            log::error!("[notia:autonomy] no se pudo preparar la corrida {}: {:?}", kind.id(), error.code);
            return;
        }
        let (run, request) = self.begin_run(job, &job.text, Vec::new());
        let silent_progress = Progress::start(self, chat_id, false, false);
        let mut question = None;
        let outcome = self.drive(&runtime, &run, request, &silent_progress, |interaction, operation| {
            // Nobody waits for an answer: a question becomes the message
            // and any other pause ends the run.
            if let crate::backend::PendingInteraction::Clarification(request) = &interaction {
                let options = request.options.iter().map(|option| format!("\n- {}", option.label)).collect::<String>();
                question = Some(format!("{}{options}", request.question.trim()));
            }
            self.cancel_operation(&run.context, &run.idempotency_key, operation.clone());
            Err("La corrida autónoma no espera respuestas.".to_string())
        });
        // The Owner wrote meanwhile: their message goes first and the next
        // run can bring this up again.
        if run.cancelled.load(Ordering::SeqCst) {
            return;
        }
        let message = match (outcome, question) {
            (Ok(response), _) => response.response.markdown,
            (Err(_), Some(question)) => question,
            // A failed run sends nothing; `drive` logged backend errors.
            (Err(_), None) => return,
        };
        if autonomy::is_silent(&message) {
            return;
        }
        self.send_markdown(chat_id, &message);
        self.keep_autonomous_message(chat_id, kind, message);
    }

    /// Runs an AI action as a turn of the Owner's chat and tells
    /// `ai_actions` how it went. Confirmations and questions reach the
    /// Owner as in any request; the answer is the message, unless the
    /// action allows silence and the agent chose it.
    fn run_action(&self, job: &Job, action: &ActionJob) {
        let context = self.database_context();
        crate::ai_actions::mark_running(&self.app, &self.library.id, &context, &action.run_id);
        let outcome = self.drive_action(job, action);
        crate::ai_actions::finish_run(&self.app, &self.library.id, &context, &action.run_id, outcome);
    }

    /// The summary of the run, or the error the dashboard shows.
    fn drive_action(&self, job: &Job, action: &ActionJob) -> Result<String, String> {
        use crate::backend::ai_actions::prompts;
        let run = self.drive_unattended(job)?;
        if prompts::is_silent(action.prompt.kind, &run.answer) {
            self.delete_messages(job.stored.chat_id, &run.notes);
            return Ok(prompts::SILENT_SUMMARY.to_string());
        }
        let message = prompts::outgoing_message(&run.answer, action.prompt.test);
        let thought = prompts::sent_thought(&action.prompt.name, &run.answer);
        self.deliver_unattended(job, run, &message, &thought).map(|answer| prompts::output_summary(&answer))
    }

    /// Runs a webhook call as a turn of the Owner's chat and tells
    /// `webhooks` how it went, as `run_action` does for an AI action.
    fn run_webhook(&self, job: &Job, webhook: &WebhookJob) {
        crate::webhooks::mark_running(&self.app, &webhook.call_id);
        let outcome = self.drive_webhook(job, webhook);
        crate::webhooks::finish_call(&self.app, &webhook.call_id, outcome);
    }

    /// The summary of the run, or the error the history of calls shows.
    fn drive_webhook(&self, job: &Job, webhook: &WebhookJob) -> Result<String, String> {
        use crate::backend::webhooks;
        let run = self.drive_unattended(job)?;
        if crate::backend::agent_autonomy::is_silent(&run.answer) {
            self.delete_messages(job.stored.chat_id, &run.notes);
            return Ok(webhooks::SILENT_SUMMARY.to_string());
        }
        let message = run.answer.trim().to_string();
        let thought = webhooks::sent_thought(&webhook.prompt.action, &message);
        self.deliver_unattended(job, run, &message, &thought).map(|answer| webhooks::output_summary(&answer))
    }

    /// Runs a turn Notia started for the Owner (an AI action, a webhook
    /// call) until the agent answers. Confirmations and questions reach the
    /// Owner as in any request.
    fn drive_unattended(&self, job: &Job) -> Result<UnattendedAnswer, String> {
        let chat_id = job.stored.chat_id;
        let runtime = self.app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
        runtime.configure_from_library_config(&self.library_config()).map_err(|error| error.message)?;
        let (run, request) = self.begin_run(job, &job.text, Vec::new());
        // Every hour a progress message would be noise: only the answer,
        // the notes before a question and the questions reach the chat.
        // A command it runs in the terminal is followed all the same.
        let progress = Progress::start(self, chat_id, false, false).with_terminal();
        let mut seen_events = 0;
        let mut notes = Vec::new();
        // Images for the person (a browser screenshot of a captcha) go
        // right before the question that asks about them.
        let mut shown = Vec::new();
        let outcome = self.drive(&runtime, &run, request, &progress, |interaction, operation| {
            notes.extend(self.deliver_notes(chat_id, &runtime, &run.context, &mut seen_events));
            shown.extend(self.send_generated_images(chat_id, &run.context.request_id));
            match self.ask(chat_id, interaction, operation) {
                Some(answer) => Ok(answer.decision),
                None if run.cancelled.load(Ordering::SeqCst) => {
                    self.cancel_operation(&run.context, &run.idempotency_key, operation.clone());
                    Err(bot::CANCELLED_MESSAGE.to_string())
                }
                None => Err("No se respondió a tiempo la confirmación o la pregunta en Telegram.".to_string()),
            }
        });
        if run.cancelled.load(Ordering::SeqCst) && outcome.is_err() {
            self.remember(chat_id, job.text.clone(), bot::CANCELLED_MESSAGE.to_string());
            return Err("La cancelaste desde Telegram.".to_string());
        }
        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                self.remember(chat_id, job.text.clone(), format!("No pude terminar: {error}"));
                return Err(error);
            }
        };
        if response.changed {
            let _ = self.app.emit(LIBRARY_CHANGED_EVENT, &self.library.id);
        }
        Ok(UnattendedAnswer { answer: response.response.markdown, request_id: run.context.request_id, notes, shown })
    }

    /// Sends `message` (the answer as the Owner reads it) and keeps the
    /// turn in the chat's history and `thought` in the agent's thoughts;
    /// the answer as the agent wrote it.
    fn deliver_unattended(&self, job: &Job, run: UnattendedAnswer, message: &str, thought: &str) -> Result<String, String> {
        let chat_id = job.stored.chat_id;
        let mut drawn = run.shown;
        drawn.extend(self.send_generated_images(chat_id, &run.request_id));
        if !self.send_markdown_checked(chat_id, message) {
            return Err("No se pudo enviar por Telegram.".to_string());
        }
        self.delete_messages(chat_id, &run.notes);
        self.remember_turn(chat_id, job.text.clone(), Vec::new(), run.answer.clone(), drawn);
        if let Err(error) = crate::agent_knowledge::keep_thought(&self.app, &self.library.id, thought) {
            log::error!("[notia:telegram] no se pudo anotar el mensaje enviado: {:?}", error.code);
        }
        Ok(run.answer)
    }

    /// Keeps a message Notia sent by itself in the chat's history and in
    /// the agent's thoughts.
    fn keep_autonomous_message(&self, chat_id: i64, kind: AutonomousKind, message: String) {
        if let Err(error) = crate::agent_knowledge::keep_thought(&self.app, &self.library.id, &autonomy::sent_thought(&message)) {
            log::error!("[notia:autonomy] no se pudo anotar el mensaje enviado: {:?}", error.code);
        }
        self.remember(chat_id, autonomy::autonomous_history_note(kind), message);
    }

    /// Context and first request of a job's run, registered as the chat's
    /// current run so a message of the chat can stop it.
    fn begin_run(&self, job: &Job, text: &str, attachments: Vec<MessageAttachment>) -> (ActiveRun, BackendRequest) {
        let chat_id = job.stored.chat_id;
        let owner = job.stored.library_user_id == OWNER;
        let context = BackendRequestContext {
            request_id: job.stored.request_id.clone(),
            library_id: self.library.id.clone(),
            actor: BackendActor {
                library_user_id: job.stored.library_user_id.clone(),
                external_identity: Some(crate::backend::context::ExternalIdentity {
                    provider: "telegram".into(),
                    user_id: job.stored.telegram_user_id.to_string(),
                    chat_id: Some(chat_id),
                }),
            },
            channel: BackendChannel::Telegram,
            // Text, photos and documents all go to the library chat, where
            // the model picks the tools the request needs.
            scope: BackendScope::Library,
            persistence_policy: if owner { PersistencePolicy::Persistent } else { PersistencePolicy::EphemeralNoMemory },
        };
        let history = self.history.lock().map(|history| history.get(&chat_id).map(|items| items.iter().cloned().collect::<Vec<_>>()).unwrap_or_default()).unwrap_or_default();
        let messages = bot::turn_messages(&history, text, attachments);
        let idempotency_key = format!("{}:{}", self.library.id, job.stored.request_id);
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = self.current.lock() {
            *current = Some(CurrentRun {
                chat_id,
                text: text.to_string(),
                context: context.clone(),
                idempotency_key: idempotency_key.clone(),
                cancelled: Arc::clone(&cancelled),
                autonomous: job.autonomous.is_some(),
            });
        }
        let request = BackendRequest::Run(AgentRequest {
            context: context.clone(),
            messages,
            snapshot: None,
            tools: Vec::new(),
            attachments: Vec::new(),
            idempotency_key: idempotency_key.clone(),
            prompt_name: None,
            tool_access: Default::default(),
            library_search: true,
            autonomous: job.autonomous.is_some(),
            scheduled_action: job.action.as_ref().map(|action| action.prompt.clone()),
            webhook_call: job.webhook.as_ref().map(|webhook| webhook.prompt.clone()),
            tool_areas: None,
            thinking: None,
            conversational: false,
            ai_task: None,
        });
        (ActiveRun { context, idempotency_key, cancelled }, request)
    }

    /// Runs `request` until the agent answers. Each pause (a confirmation,
    /// plan or question) is resumed with what `pause` decides, or ends the
    /// run with its error. A cancelled run ends with the cancel message.
    fn drive(
        &self,
        runtime: &crate::backend_runtime::BackendRuntimeState,
        run: &ActiveRun,
        mut request: BackendRequest,
        progress: &Progress,
        mut pause: impl FnMut(crate::backend::PendingInteraction, &crate::backend::OperationToken) -> Result<ResumeDecision, String>,
    ) -> Result<crate::backend::AgentResponse, String> {
        loop {
            let running = Arc::new(AtomicBool::new(true));
            let observer = progress.observe(runtime, &run.context, Arc::clone(&running));
            let result = {
                let registry = self.app.state::<crate::library_registry::LibraryBindingRegistry>();
                crate::backend_runtime::execute_backend_request(
                    &self.app,
                    BackendRequestEnvelope { protocol_version: ProtocolVersion::default(), request },
                    runtime,
                    registry.inner(),
                )
            };
            running.store(false, Ordering::SeqCst);
            if let Some(observer) = observer {
                let _ = observer.join();
            }
            let response = match result {
                Ok(envelope) => envelope.response,
                Err(error) => {
                    crate::backend_runtime::log_request_failure(runtime, "telegram", &run.context, &error);
                    return Err(error.message);
                }
            };
            let stopped = run.cancelled.load(Ordering::SeqCst);
            match response {
                // The run finished before the cancel reached it.
                BackendResponse::Result { response } => return Ok(response),
                BackendResponse::Operation { status } | BackendResponse::Resumed { status, .. } if stopped => {
                    if let Some(operation) = status.operation {
                        self.cancel_operation(&run.context, &run.idempotency_key, operation);
                    }
                    return Err(bot::CANCELLED_MESSAGE.to_string());
                }
                _ if stopped => return Err(bot::CANCELLED_MESSAGE.to_string()),
                BackendResponse::Error { error, .. } => {
                    crate::backend_runtime::log_request_failure(runtime, "telegram", &run.context, &error);
                    return Err(error.message);
                }
                BackendResponse::Operation { status } | BackendResponse::Resumed { status, .. } => {
                    let (Some(operation), Some(interaction)) = (status.operation.clone(), status.interaction.clone()) else {
                        return Err("El runtime no devolvió una respuesta final.".to_string());
                    };
                    let decision = pause(interaction, &operation)?;
                    request = BackendRequest::Resume(ResumeRequest {
                        context: run.context.clone(),
                        idempotency_key: run.idempotency_key.clone(),
                        request_id: run.context.request_id.clone(),
                        operation,
                        last_event_sequence: status.last_event_sequence,
                        decision,
                    });
                }
                _ => return Err("El runtime no devolvió una respuesta final.".to_string()),
            }
        }
    }

    /// Adds a request and its answer to the chat's recent history.
    fn remember(&self, chat_id: i64, request: String, answer: String) {
        self.remember_with_files(chat_id, request, Vec::new(), answer);
    }

    /// Adds a request with its files (photos, documents) and its answer to
    /// the chat's recent history; later requests show the files on this
    /// request, not as new ones.
    fn remember_with_files(&self, chat_id: i64, request: String, files: Vec<MessageAttachment>, answer: String) {
        self.remember_turn(chat_id, request, files, answer, Vec::new());
    }

    /// Like `remember_with_files`, with the images the agent sent with its
    /// answer, so a later request can edit them by their reference. Telegram
    /// is ephemeral: they live only in this history.
    fn remember_turn(&self, chat_id: i64, request: String, files: Vec<MessageAttachment>, answer: String, drawn: Vec<MessageAttachment>) {
        if let Ok(mut history) = self.history.lock() {
            let entries = history.entry(chat_id).or_default();
            entries.push_back(BackendMessage { role: MessageRole::User, content: request, images: Vec::new(), attachments: files });
            entries.push_back(BackendMessage { role: MessageRole::Assistant, content: answer, images: Vec::new(), attachments: drawn });
            while entries.len() > bot::MAX_HISTORY_MESSAGES {
                entries.pop_front();
            }
        }
    }

    /// Marks the operation a cancelled run was waiting on as cancelled.
    fn cancel_operation(&self, context: &BackendRequestContext, idempotency_key: &str, operation: crate::backend::OperationToken) {
        let runtime = self.app.state::<crate::backend_runtime::BackendRuntimeState>().inner().clone();
        let registry = self.app.state::<crate::library_registry::LibraryBindingRegistry>();
        let _ = crate::backend_runtime::execute_backend_request(
            &self.app,
            cancel_envelope(context, idempotency_key, Some(operation)),
            &runtime,
            registry.inner(),
        );
    }

    /// Shows the pending interaction and waits for the person's answer: the
    /// decision the run resumes with and, when the work goes on, the text
    /// that opens the next progress message. Once answered (or expired), the
    /// question's message loses its buttons and shows the answer; answered
    /// with a button, the progress goes on in that same message.
    fn ask(
        &self,
        chat_id: i64,
        interaction: crate::backend::PendingInteraction,
        operation: &crate::backend::OperationToken,
    ) -> Option<Answer> {
        use crate::backend::PendingInteraction as Interaction;
        use bot::ConfirmationReply as Choice;
        if self.cancel_requested(chat_id) {
            return None;
        }
        let (sender, receiver) = mpsc::channel();
        let choice_buttons = |id: &str| {
            bot::CONFIRMATION_BUTTONS
                .iter()
                .map(|(label, reply)| (label.to_string(), format!("confirm:{id}:{}", reply.id())))
                .collect::<Vec<_>>()
        };
        let (prompt, timeout, sent) = match &interaction {
            Interaction::Confirmation(request) => {
                let id = short_id()[..8].to_string();
                let sent = self.send_parts(chat_id, bot::confirmation_parts(&request.preview), choice_buttons(&id));
                (Prompt::Confirmation { id }, CONFIRMATION_TIMEOUT, sent)
            }
            Interaction::Plan(plan) => {
                let id = short_id()[..8].to_string();
                let sent = self.send_parts(chat_id, bot::plan_parts(plan), choice_buttons(&id));
                (Prompt::Confirmation { id }, CONFIRMATION_TIMEOUT, sent)
            }
            Interaction::Clarification(request) => {
                let choices = request.options.iter().map(|option| option.label.clone()).collect::<Vec<_>>();
                let buttons = choices
                    .iter()
                    .enumerate()
                    .map(|(index, label)| (label.chars().take(bot::MAX_BUTTON_CHARS).collect(), format!("choice:{index}")))
                    .collect();
                let sent = self.send_parts(chat_id, bot::question_parts(&request.question, &choices), buttons);
                (Prompt::Clarification { choices }, CLARIFICATION_TIMEOUT, sent)
            }
        };
        // The question's message without its buttons, with `outcome` in
        // place of its choices; its id and new text.
        let answered = |outcome: &str| {
            sent.as_ref().map(|(id, html)| {
                let text = bot::answered_prompt(html, outcome);
                self.edit_html(chat_id, *id, &text);
                (*id, text)
            })
        };
        // A cancel dropped the question: the run stops without an answer.
        let Some(reply) = self.wait_reply(chat_id, prompt, &sender, &receiver, timeout) else {
            answered("Solicitud cancelada.");
            return None;
        };
        // «Proponer otra cosa»: the proposal is the next message.
        let (reply, proposal, message) = match reply {
            Some(Reply::Decision(Choice::Propose, _)) => {
                answered(bot::confirmation_outcome(Some(Choice::Propose)));
                self.send(chat_id, "Escribí qué querés que haga en su lugar.");
                match self.wait_reply(chat_id, Prompt::Proposal, &sender, &receiver, CLARIFICATION_TIMEOUT)? {
                    Some(Reply::Text(text, _, _)) if !text.trim().is_empty() => (Some(Choice::Propose), Some(text.trim().to_string()), None),
                    _ => (None, None, None),
                }
            }
            Some(Reply::Decision(choice, tapped)) => {
                let message = answered(bot::confirmation_outcome(Some(choice)));
                (Some(choice), None, message.filter(|_| tapped))
            }
            Some(Reply::Text(answer, index, tapped)) => {
                let Interaction::Clarification(request) = interaction else {
                    return None;
                };
                let message = answered(&bot::question_outcome(Some(&answer)));
                return Some(Answer {
                    decision: ResumeDecision::Clarification(crate::backend::ClarificationAnswer {
                        clarification_id: request.clarification_id.clone(),
                        operation: operation.clone(),
                        answer,
                        option_id: index.and_then(|index| request.options.get(index)).map(|option| option.id.clone()),
                    }),
                    acknowledgement: Some("Respuesta recibida. Sigo con tu pedido…".to_string()),
                    message: message.filter(|_| tapped),
                });
            }
            None => {
                let expired = match interaction {
                    Interaction::Clarification(_) => bot::question_outcome(None),
                    _ => bot::confirmation_outcome(None).to_string(),
                };
                answered(&expired);
                (None, None, None)
            }
        };
        let approve_all = reply == Some(Choice::ConfirmAll);
        let accepted = matches!(reply, Some(Choice::Confirm | Choice::ConfirmAll));
        match interaction {
            Interaction::Confirmation(_) => {
                let acknowledgement = match reply {
                    Some(Choice::Confirm) => Some("Confirmación recibida. Aplicando el cambio…"),
                    Some(Choice::ConfirmAll) => Some("Confirmado todo lo que queda de este pedido. Sigo sin volver a preguntarte…"),
                    Some(Choice::Propose) => Some("Propuesta recibida. Sigo con lo que me pediste…"),
                    // The agent's answer says the change was rejected; an
                    // expired confirmation says so on its message.
                    Some(Choice::Cancel) | None => None,
                };
                Some(Answer {
                    decision: ResumeDecision::Confirmation(crate::backend::ConfirmationDecision {
                        operation_id: operation.operation_id.clone(),
                        accepted,
                        hunk_ids: Vec::new(),
                        approve_all,
                        suggestion: proposal,
                    }),
                    acknowledgement: acknowledgement.map(str::to_string),
                    message,
                })
            }
            Interaction::Plan(plan) => {
                let acknowledgement = match reply {
                    Some(Choice::Confirm) => Some("Plan aprobado. Empiezo…"),
                    Some(Choice::ConfirmAll) => Some("Plan aprobado junto con todo lo que siga en este pedido. Empiezo sin volver a preguntarte…"),
                    Some(Choice::Propose) => Some("Propuesta recibida. Rehago el plan…"),
                    Some(Choice::Cancel) | None => None,
                };
                Some(Answer {
                    decision: ResumeDecision::Plan(crate::backend::PlanDecision {
                        plan_id: plan.plan_id.clone(),
                        generation: plan.generation,
                        accepted,
                        step_ids: plan.steps.iter().map(|step| step.id.clone()).collect(),
                        suggestion: proposal,
                        approve_all,
                    }),
                    acknowledgement: acknowledgement.map(str::to_string),
                    message,
                })
            }
            // Buttons answer confirmations and plans; a question needs text.
            Interaction::Clarification(_) => None,
        }
    }

    /// Waits for the answer to `prompt`, or `None` inside when it timed out.
    /// `None` outside means a cancel dropped the question.
    fn wait_reply(
        &self,
        chat_id: i64,
        prompt: Prompt,
        sender: &mpsc::Sender<Reply>,
        receiver: &mpsc::Receiver<Reply>,
        timeout: Duration,
    ) -> Option<Option<Reply>> {
        if let Ok(mut pending) = self.prompt.lock() {
            *pending = Some((chat_id, prompt, sender.clone()));
        }
        // A cancel that arrived while the question was being sent drops it.
        let reply = if self.cancel_requested(chat_id) { None } else { receiver.recv_timeout(timeout).ok() };
        if let Ok(mut pending) = self.prompt.lock() {
            *pending = None;
        }
        (!self.cancel_requested(chat_id)).then_some(reply)
    }
}

/// The messages that follow the commands of the terminal in the chat, one
/// per command: sent when it starts, edited with what it prints, posted
/// again below every [`bot::TERMINAL_REPOST_INTERVAL`] while it runs (so
/// the person gets a notification each minute, even when the agent waits
/// for a long command) and left with how it ended.
#[derive(Default)]
struct TerminalMessages {
    by_call: HashMap<String, TerminalMessage>,
}

struct TerminalMessage {
    message_id: Option<i64>,
    started: Instant,
    posted: Instant,
    edited: Instant,
    shown: String,
    ended: bool,
}

/// A running command's message is edited at most this often: Telegram
/// limits the edits of a chat.
const TERMINAL_EDIT_INTERVAL: Duration = Duration::from_secs(5);

impl TerminalMessages {
    fn update(&mut self, token: &str, chat_id: i64, runs: &[notia_backend_core::terminal::TerminalRun]) {
        for run in runs {
            let now = Instant::now();
            let message = self.by_call.entry(run.call_id.clone()).or_insert_with(|| TerminalMessage {
                message_id: None,
                started: now,
                posted: now,
                edited: now,
                shown: String::new(),
                ended: false,
            });
            if message.ended {
                continue;
            }
            let ended = run.ended.is_some();
            let html = bot::terminal_message(run, message.started.elapsed());
            match message.message_id {
                Some(id) if !ended && message.posted.elapsed() >= bot::TERMINAL_REPOST_INTERVAL => {
                    let _ = block_on(telegram::delete_message(token, chat_id, id));
                    message.message_id = block_on(telegram::send_message(token, chat_id, &html, Vec::new(), Some("HTML"))).ok();
                    message.posted = now;
                    message.edited = now;
                }
                Some(id) if html != message.shown && (ended || message.edited.elapsed() >= TERMINAL_EDIT_INTERVAL) => {
                    let _ = block_on(telegram::edit_message(token, chat_id, id, &html, Vec::new(), Some("HTML")));
                    message.edited = now;
                }
                Some(_) => {
                    message.ended = ended;
                    continue;
                }
                None => {
                    message.message_id = block_on(telegram::send_message(token, chat_id, &html, Vec::new(), Some("HTML"))).ok();
                    message.posted = now;
                    message.edited = now;
                }
            }
            message.shown = html;
            message.ended = ended;
        }
    }
}

/// Longest question text (HTML) the progress goes on below; a longer one
/// leaves no room for the progress in a Telegram message.
const MAX_ADOPTED_TEXT_CHARS: usize = 3_400;

/// Progress message of a running request, edited while events arrive
/// (or sent anew when the user turned editing off). After a question
/// answered with a button, the progress message is the question's: its
/// text stays above the progress (`header`) and remains once the work ends.
struct Progress {
    token: String,
    chat_id: i64,
    enabled: bool,
    edit: bool,
    message_id: Arc<Mutex<Option<i64>>>,
    header: Arc<Mutex<Option<String>>>,
    last_sequence: Arc<Mutex<u64>>,
    events: Arc<Mutex<Vec<crate::backend::BackendEvent>>>,
    /// Whether the commands of the terminal get their own messages, even
    /// with the progress off: the person follows what runs on the PC.
    terminal: bool,
    terminal_messages: Arc<Mutex<TerminalMessages>>,
}

impl Progress {
    /// Follows the commands the agent runs in the terminal (see
    /// [`TerminalMessages`]).
    fn with_terminal(mut self) -> Progress {
        self.terminal = true;
        self
    }

    fn start(worker: &Worker, chat_id: i64, enabled: bool, edit: bool) -> Progress {
        let message_id = if enabled { worker.send_html(chat_id, "<b>Preparando la solicitud</b>", Vec::new()) } else { None };
        Progress {
            token: worker.token.clone(),
            chat_id,
            enabled,
            edit,
            message_id: Arc::new(Mutex::new(message_id)),
            header: Arc::new(Mutex::new(None)),
            last_sequence: Arc::new(Mutex::new(0)),
            events: Arc::new(Mutex::new(Vec::new())),
            terminal: false,
            terminal_messages: Arc::new(Mutex::new(TerminalMessages::default())),
        }
    }

    /// Polls the request's events while it runs and updates the message.
    fn observe(
        &self,
        runtime: &crate::backend_runtime::BackendRuntimeState,
        context: &BackendRequestContext,
        running: Arc<AtomicBool>,
    ) -> Option<std::thread::JoinHandle<()>> {
        if !self.enabled && !self.terminal {
            return None;
        }
        let runtime = runtime.clone();
        let context = context.clone();
        let message_id = Arc::clone(&self.message_id);
        let header = Arc::clone(&self.header);
        let last_sequence = Arc::clone(&self.last_sequence);
        let events = Arc::clone(&self.events);
        let terminal_messages = Arc::clone(&self.terminal_messages);
        let (token, chat_id, edit, enabled, terminal) = (self.token.clone(), self.chat_id, self.edit, self.enabled, self.terminal);
        let token_for_terminal = self.token.clone();
        // Reads the new events; with them, the progress text and the
        // commands of the terminal.
        let read = move || -> Option<(Option<String>, Vec<notia_backend_core::terminal::TerminalRun>)> {
            let after = last_sequence.lock().map(|value| *value).unwrap_or_default();
            let batch = runtime.request_events(&context, after).ok()?;
            let mut seen = events.lock().ok()?;
            for envelope in batch {
                if let Ok(mut last) = last_sequence.lock() {
                    *last = (*last).max(envelope.sequence);
                }
                // Stream fragments do not change the progress message and a
                // long answer has thousands; a command's output joins.
                bot::keep_progress_event(&mut seen, envelope.event);
            }
            Some((bot::progress_message(&seen), notia_backend_core::terminal::terminal_runs(&seen)))
        };
        let follow_terminal = move |runs: &[notia_backend_core::terminal::TerminalRun]| {
            if terminal {
                if let Ok(mut messages) = terminal_messages.lock() {
                    messages.update(&token_for_terminal, chat_id, runs);
                }
            }
        };
        std::thread::Builder::new()
            .name("notia-telegram-progress".into())
            .spawn(move || {
                let mut shown = String::new();
                while running.load(Ordering::SeqCst) {
                    std::thread::sleep(PROGRESS_INTERVAL);
                    let Some((message, runs)) = read() else {
                        continue;
                    };
                    follow_terminal(&runs);
                    let Some(message) = message.filter(|message| enabled && *message != shown) else {
                        continue;
                    };
                    shown = message.clone();
                    let current = message_id.lock().ok().and_then(|value| *value);
                    match current.filter(|_| edit) {
                        Some(id) => {
                            let text = match header.lock().ok().and_then(|value| value.clone()) {
                                Some(header) => format!("{header}\n\n{message}"),
                                None => message,
                            };
                            let _ = block_on(telegram::edit_message(&token, chat_id, id, &text, Vec::new(), Some("HTML")));
                        }
                        None => {
                            let sent = block_on(telegram::send_message(&token, chat_id, &message, Vec::new(), Some("HTML"))).ok();
                            if let Ok(mut value) = message_id.lock() {
                                *value = sent;
                            }
                        }
                    }
                }
                // A command that ended with the run still shows how.
                if let Some((_, runs)) = read() {
                    follow_terminal(&runs);
                }
            })
            .ok()
    }

    /// Opens a new progress message with `text` below the person's answer,
    /// so the work shows at the end of the chat, and deletes the one above.
    /// Without an edited progress message, `text` goes as a plain message.
    fn restart(&self, worker: &Worker, text: &str) {
        if !self.enabled || !self.edit {
            worker.send(self.chat_id, text);
            return;
        }
        self.release(worker);
        let sent = worker.send_html(self.chat_id, &format!("<b>{}</b>", crate::backend::escape_telegram_html(text)), Vec::new());
        if let Ok(mut id) = self.message_id.lock() {
            *id = sent;
        }
        // The new message shows only what happens from now on.
        if let Ok(mut events) = self.events.lock() {
            events.clear();
        }
    }

    /// Goes on in the message of a question answered with a button: its
    /// `text` (HTML, already without buttons) stays above the progress,
    /// which starts with `acknowledgement`. The progress message above is
    /// let go. Without room below the text, or without an edited progress
    /// message, the progress restarts below instead.
    fn adopt(&self, worker: &Worker, message_id: i64, text: String, acknowledgement: Option<&str>) {
        if !self.enabled || !self.edit || text.chars().count() > MAX_ADOPTED_TEXT_CHARS {
            if let Some(acknowledgement) = acknowledgement {
                self.restart(worker, acknowledgement);
            }
            return;
        }
        if self.message_id.lock().ok().and_then(|value| *value) != Some(message_id) {
            self.release(worker);
        }
        if let Some(acknowledgement) = acknowledgement {
            worker.edit_html(self.chat_id, message_id, &format!("{text}\n\n<b>{}</b>", crate::backend::escape_telegram_html(acknowledgement)));
        }
        if let Ok(mut id) = self.message_id.lock() {
            *id = Some(message_id);
        }
        if let Ok(mut header) = self.header.lock() {
            *header = Some(text);
        }
        if let Ok(mut events) = self.events.lock() {
            events.clear();
        }
    }

    /// Lets the current progress message go: a question's message keeps
    /// only its text and answer; a plain progress message is deleted.
    fn release(&self, worker: &Worker) {
        let header = self.header.lock().ok().and_then(|mut value| value.take());
        let Some(id) = self.message_id.lock().ok().and_then(|mut value| value.take()) else {
            return;
        };
        match header {
            Some(header) => worker.edit_html(self.chat_id, id, &header),
            None => {
                let _ = block_on(telegram::delete_message(&self.token, self.chat_id, id));
            }
        }
    }

    /// Clears the progress, as the answer comes next.
    fn dismiss(&self, worker: &Worker) {
        if self.enabled && self.edit {
            self.release(worker);
        }
    }

    /// Leaves the progress message with the outcome `text` (HTML), below the
    /// question it went on in, if any.
    fn finish(&self, worker: &Worker, text: &str) {
        let Some(id) = self.message_id.lock().ok().and_then(|value| *value).filter(|_| self.enabled && self.edit) else {
            return;
        };
        match self.header.lock().ok().and_then(|value| value.clone()) {
            Some(header) => worker.edit_html(self.chat_id, id, &format!("{header}\n\n{text}")),
            None => {
                let _ = block_on(telegram::edit_message(&self.token, self.chat_id, id, text, Vec::new(), Some("HTML")));
            }
        }
    }
}

fn cancel_envelope(
    context: &BackendRequestContext,
    idempotency_key: &str,
    operation: Option<crate::backend::OperationToken>,
) -> BackendRequestEnvelope {
    BackendRequestEnvelope {
        protocol_version: ProtocolVersion::default(),
        request: BackendRequest::Cancel(crate::backend::CancelRequest {
            context: context.clone(),
            idempotency_key: idempotency_key.to_string(),
            request_id: context.request_id.clone(),
            operation,
        }),
    }
}

/// What a failed request did before failing: nothing (it can be sent
/// again) or some actions (check them before repeating it).
fn failure_progress(runtime: &crate::backend_runtime::BackendRuntimeState, context: &BackendRequestContext) -> &'static str {
    let ran = runtime
        .request_events(context, 0)
        .unwrap_or_default()
        .iter()
        .any(|envelope| matches!(envelope.event, crate::backend::BackendEvent::ToolCompleted { .. }));
    if ran {
        "Algunas acciones se hicieron antes del error: preguntame «¿qué hiciste?» antes de repetir el pedido."
    } else {
        "No llegué a hacer ninguna acción: podés repetir el pedido."
    }
}

fn strip_html(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn audio_file_id(audio: &telegram::TelegramAudio) -> String {
    serde_json::to_value(audio)
        .ok()
        .and_then(|value| value.get("fileId").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn transcribe(app: &AppHandle, token: &str, audio: &telegram::TelegramAudio) -> Result<String, String> {
    let state = app.state::<crate::services::speech_service::SpeechRuntimeState>();
    if state.phase.lock().map_err(|_| "No se pudo consultar el estado de voz.".to_string())?.is_active() {
        return Err("El dictado local está usando el reconocedor de voz; reenviá el audio en unos segundos.".to_string());
    }
    let bytes = block_on(telegram::download_audio(token, audio))?;
    let recognizer = crate::services::speech_service::recognizer_cache(&state);
    let samples = crate::services::telegram_audio::decode_telegram_ogg_opus(bytes)?;
    crate::services::speech_service::transcribe_external_audio(app, &recognizer, &samples)
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
fn transcribe(_app: &AppHandle, _token: &str, _audio: &telegram::TelegramAudio) -> Result<String, String> {
    Err("La transcripción de audios de Telegram no está disponible en esta plataforma.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_jobs_never_keep_the_request_text() {
        let job = StoredJob {
            request_id: "r".into(),
            chat_id: 1,
            telegram_user_id: 2,
            library_user_id: OWNER.into(),
            attachments: Vec::new(),
            attachment: None,
        };
        let text = serde_json::to_string(&WorkerFile { offset: 3, processed: VecDeque::from([1, 2]), jobs: vec![job] }).expect("json");
        assert!(!text.contains("text"));
        let restored: WorkerFile = serde_json::from_str(&text).expect("restore");
        assert_eq!((restored.offset, restored.jobs.len()), (3, 1));
    }

    #[test]
    fn a_queue_saved_by_an_older_version_keeps_its_file() {
        let saved = r#"{"offset":1,"jobs":[{"requestId":"r","chatId":1,"telegramUserId":2,"libraryUserId":"user-owner","attachment":{"kind":"pdf","value":{"fileId":"f","fileName":"resumen.pdf"}},"finance":true}]}"#;
        let mut file: WorkerFile = serde_json::from_str(saved).expect("older queue");
        file.jobs.iter_mut().for_each(StoredJob::adopt_legacy_attachment);
        let job = &file.jobs[0];
        assert!(job.attachment.is_none());
        assert!(matches!(job.attachments.as_slice(), [JobAttachment::Document(document)] if document.file_id == "f"));
        let text = serde_json::to_string(&file).expect("json");
        assert!(text.contains("\"attachments\":[{\"kind\":\"document\"") && !text.contains("\"attachment\":") && !text.contains("finance"));
    }

    #[test]
    fn messages_written_line_by_line_become_one_request_after_the_last_wait() {
        let mut batches = HashMap::new();
        let wait = Duration::from_secs(150);
        let photo = || {
            JobAttachment::Document(TelegramDocument { file_id: "f".into(), file_name: Some("ticket.pdf".into()), mime_type: None, file_size: None })
        };
        assert!(add_to_batch(&mut batches, 1, 2, OWNER.into(), "hola".into(), Vec::new(), wait));
        let first_ready = batches[&1].ready_at;
        std::thread::sleep(Duration::from_millis(5));
        assert!(!add_to_batch(&mut batches, 1, 2, OWNER.into(), "fijate el ticket".into(), vec![photo()], wait));
        assert!(!add_to_batch(&mut batches, 1, 2, OWNER.into(), "  ".into(), Vec::new(), Duration::from_secs(120)));
        // Each message starts the wait again, from itself.
        assert!(batches[&1].ready_at > first_ready - Duration::from_secs(30));
        assert!(batches[&1].ready_at <= Instant::now() + Duration::from_secs(120));
        // Another chat waits on its own.
        assert!(add_to_batch(&mut batches, 7, 8, OWNER.into(), "otro".into(), Vec::new(), wait));
        let batch = &batches[&1];
        assert_eq!(batch.text(), "hola\nfijate el ticket");
        assert_eq!(batch.attachments.len(), 1);
        let stored = batch.stored(1);
        assert_eq!((stored.chat_id, stored.request_id.as_str(), stored.attachments.len()), (1, batch.request_id.as_str(), 1));
        for _ in 0..MAX_ALBUM_PARTS + 2 {
            add_to_batch(&mut batches, 1, 2, OWNER.into(), String::new(), vec![photo()], wait);
        }
        assert_eq!(batches[&1].attachments.len(), MAX_ALBUM_PARTS);
    }

    #[test]
    fn html_fallback_keeps_the_words() {
        assert_eq!(strip_html("<b>Hola</b> &lt;x&gt; &amp; más"), "Hola <x> & más");
    }
}
