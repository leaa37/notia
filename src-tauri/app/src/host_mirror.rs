//! The copy a client keeps of its host's library, which is the library the
//! client works on.
//!
//! - **Always on the copy.** The window of a client opens the copy as a
//!   library of this device (`open_copy`), whether the host answers or not:
//!   notes, files and the database change here, and the AI and speech
//!   recognition are local.
//! - **Synced in the background** whenever the host answers and accepts
//!   the Owner's session: the monitor of `host_client` starts a sync on
//!   every check (`sync_in_background`), with the rules of `mirror_sync`
//!   and the commands of `host_sync`. The window never waits for it.
//! - **Both sides change**: the last modified file wins, and in the
//!   database the last change of each row wins (`row_sync`), whatever
//!   device made it. What changed on this device travels first. After a
//!   sync that brought changes, the window is told to read them again.
//!
//! The database is copied whole once (a snapshot of the host); after that
//! only the rows that changed travel, both ways, on every sync.
//!
//! Where the copy lives (`CopyStore`): on Windows and Linux in the app data
//! folder (`mirror/<library>/<name>`); on Android in a folder the person
//! chooses (a SAF tree), because Android libraries are SAF folders. The
//! record of the last sync stays in the app data folder, and a mark in the
//! copy (`.notia/notia-copy.json`) keeps another library from mixing in.
//!
//! The first copy of a library can take minutes on a phone. The record is
//! saved as the sync goes (`Pass::save`), with each file as the copy's
//! folder has it after writing it, so a sync cut short (the app closed or
//! was frozen) goes on where it stopped instead of starting over. While a
//! long sync runs, Android keeps the process going with a foreground
//! service (`KeepRunning`), and the status shows how far it went.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::backend::mirror_sync::{plan, FileStamp, SyncAction, SyncBase, COPY_MARK_PATH};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Emitter, Manager};
use crate::host_sync::{
    delete_synced, synced_files, synced_target, write_synced, DatabaseChanges, DatabaseSnapshot, SyncFile, SyncManifest,
    MAX_ROW_CHANGES, MAX_SYNC_UPLOAD_BYTES,
};
use crate::row_sync::ApplyReport;

/// Event of the interface after each sync of the copy.
pub(crate) const COPY_EVENT: &str = "notia:copy-sync";
const DIRECTORY: &str = "mirror";
const CURRENT_FILE: &str = "current.json";
/// Mark of a copy, with the host library it belongs to.
const MARK_PATH: &str = COPY_MARK_PATH;
const DATABASE_PATH: &str = ".notia/notia.db";
/// How often a running sync saves its record and shows how far it went.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
/// A sync with at least this many files keeps an Android process running
/// with the window hidden; shorter ones end in a few seconds.
const LONG_SYNC_FILES: usize = 20;

#[derive(Default)]
pub(crate) struct MirrorState {
    sync: tokio::sync::Mutex<()>,
    /// A sync of `sync_in_background` runs.
    background: AtomicBool,
    /// The running sync moves files or the database (not only compares).
    transferring: AtomicBool,
    /// Files done and to do of the running sync.
    progress: Mutex<Option<(usize, usize)>>,
    /// The copy is open as a library of this device.
    open: AtomicBool,
    /// Android, where listing a SAF folder is slow: how many library writes
    /// this device had made when the copy was last listed. A sync lists it
    /// again only after a new write; otherwise it trusts the record.
    listed_at: Mutex<Option<u64>>,
    last: Mutex<Option<CopyStatus>>,
}

/// The copy the window works on: its database opens only when it holds
/// the host's tables (`database::open_copy_database_file`).
enum CopyLocation {
    #[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
    Folder(PathBuf),
    #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
    Saf(String),
}

static OPEN_COPY: Mutex<Option<CopyLocation>> = Mutex::new(None);

/// Whether the library at `library_path` is the copy the window works on
/// (Windows, Linux and macOS).
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(crate) fn database_is_copy(library_path: &str) -> bool {
    match OPEN_COPY.lock().ok().as_deref() {
        Some(Some(CopyLocation::Folder(root))) => std::fs::canonicalize(library_path).is_ok_and(|path| &path == root),
        _ => false,
    }
}

/// Whether the SAF library `tree_uri` is the copy the window works on
/// (Android).
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub(crate) fn database_uri_is_copy(tree_uri: &str) -> bool {
    matches!(OPEN_COPY.lock().ok().as_deref(), Some(Some(CopyLocation::Saf(uri))) if uri == tree_uri)
}

/// Record of the copy of one library of the host.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopyRecord {
    /// Folder of the copy (Windows and Linux).
    #[serde(default)]
    folder: Option<PathBuf>,
    /// SAF folder of the copy (Android); another folder starts over.
    #[serde(default)]
    folder_uri: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, SyncBase>,
    #[serde(default)]
    database_ms: Option<i64>,
    /// The host database the copy follows, and the last of its row changes
    /// the copy took (`row_sync`).
    #[serde(default)]
    database_id: Option<String>,
    #[serde(default)]
    database_seq: i64,
    /// The library of this device the copy opens as.
    #[serde(default)]
    local_library_id: Option<String>,
    /// The library selected on this device before the copy opened, for
    /// going back to it in Host mode.
    #[serde(default)]
    previous_selection: Option<String>,
}

/// The host library this device copies, kept for opening the copy at start.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentCopy {
    host_address: String,
    library_id: String,
    library_name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopyMark {
    host_library_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyStatus {
    downloaded: usize,
    uploaded: usize,
    deleted: usize,
    /// Rows of the database this device changed that the host took, and
    /// rows the host changed that the copy took.
    rows_uploaded: usize,
    rows_downloaded: usize,
    /// Files that did not travel (too large, or the host did not give them).
    skipped: Vec<String>,
    at_ms: i64,
    error: Option<String>,
}

fn storage(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, message, true)
}

fn unavailable(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Unsupported, message, false)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn directory(app: &AppHandle) -> Result<PathBuf, BackendError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(DIRECTORY))
        .map_err(|_| storage("No se encontró la carpeta de datos de Notia."))
}

/// A name that is safe as a single folder name.
fn folder_name(text: &str, fallback: &str) -> String {
    let name: String = text
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .chars()
        .take(80)
        .collect();
    if name.is_empty() { fallback.to_string() } else { name }
}

fn record_path(app: &AppHandle, library_id: &str) -> Result<PathBuf, BackendError> {
    Ok(directory(app)?.join(format!("{}.json", folder_name(library_id, "biblioteca"))))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), BackendError> {
    let failed = || storage("No se pudo guardar el estado de la copia.");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| failed())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|_| failed())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|_| failed())?;
    std::fs::rename(&temporary, path).map_err(|_| failed())
}

fn state(app: &AppHandle) -> Option<crate::host::State<'static, MirrorState>> {
    app.try_state::<MirrorState>()
}

/// Whether this device keeps a copy of its host's library: every client does.
fn keeps_copy(app: &AppHandle) -> bool {
    crate::connection::is_client(app)
}

/// Id and name of the host library this device copied last.
pub(crate) fn copied_library(app: &AppHandle) -> Option<(String, String)> {
    let current: CurrentCopy = read_json(&directory(app).ok()?.join(CURRENT_FILE));
    (!current.library_id.is_empty()).then_some((current.library_id, current.library_name))
}

/// The last sync, for Settings and the window.
pub(crate) fn last_status(app: &AppHandle) -> Option<CopyStatus> {
    state(app).and_then(|state| state.last.lock().ok().and_then(|last| last.clone()))
}

/// Whether a sync is moving files or the database right now.
pub(crate) fn is_transferring(app: &AppHandle) -> bool {
    state(app).is_some_and(|state| state.transferring.load(Ordering::SeqCst))
}

/// Files done and to do of the running sync.
pub(crate) fn progress(app: &AppHandle) -> Option<(usize, usize)> {
    state(app).and_then(|state| state.progress.lock().ok().and_then(|progress| *progress))
}

/// Files that did not travel in the last sync (they are tried again).
pub(crate) fn last_skipped(app: &AppHandle) -> usize {
    last_status(app).map_or(0, |status| status.skipped.len())
}

/// The error of the last sync, when it failed.
pub(crate) fn last_error(app: &AppHandle) -> Option<String> {
    last_status(app).and_then(|status| status.error)
}

/// Whether this device has a copy of its host's library to open: synced at
/// least once with the saved host, with its database.
pub(crate) fn copy_ready(app: &AppHandle) -> bool {
    if !keeps_copy(app) {
        return false;
    }
    let Ok(directory) = directory(app) else {
        return false;
    };
    let current: CurrentCopy = read_json(&directory.join(CURRENT_FILE));
    if current.library_id.is_empty() || current.host_address != crate::connection::settings(app).host_address {
        return false;
    }
    let Ok(record_file) = record_path(app, &current.library_id) else {
        return false;
    };
    let record: CopyRecord = read_json(&record_file);
    record.database_ms.is_some() && copy_folder_exists(app, &record)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn copy_folder_exists(_app: &AppHandle, record: &CopyRecord) -> bool {
    record.folder.as_ref().is_some_and(|folder| folder.is_dir())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn copy_folder_exists(app: &AppHandle, record: &CopyRecord) -> bool {
    crate::connection::settings(app).copy_folder.is_some_and(|folder| record.folder_uri.as_deref() == Some(folder.uri.as_str()))
}

// ---------- Where the copy lives ----------

/// The folder of the copy and how its files are read and written.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
enum CopyStore {
    /// A folder of the app data (Windows and Linux), by canonical path.
    Folder(PathBuf),
    /// The SAF folder the person chose (Android). Its tree URI is kept as
    /// received; files resolve to their own document URIs. `folders`
    /// remembers the document URI of each folder written in this sync, so
    /// a file is created with one query instead of one per folder.
    Saf { tree_uri: String, folders: Mutex<BTreeMap<String, String>> },
}

fn copy_read_error() -> BackendError {
    storage("No se pudo leer un archivo de la copia.")
}

fn copy_write_error() -> BackendError {
    storage("No se pudo guardar un archivo en la copia.")
}

impl CopyStore {
    /// Files of the copy that travel, with their size and time.
    fn list(&self, app: &AppHandle) -> Result<BTreeMap<String, FileStamp>, BackendError> {
        match self {
            Self::Folder(root) => Ok(synced_files(root)?.into_iter().map(|file| (file.path, file.stamp)).collect()),
            Self::Saf { tree_uri, .. } => saf::list(app, tree_uri),
        }
    }

    fn read(&self, app: &AppHandle, path: &str) -> Result<Vec<u8>, BackendError> {
        match self {
            Self::Folder(root) => std::fs::read(synced_target(root, path)?).map_err(|_| copy_read_error()),
            Self::Saf { tree_uri, .. } => saf::read(app, tree_uri, path)?.ok_or_else(copy_read_error),
        }
    }

    /// Writes a file that travels (a desktop folder keeps the host's time)
    /// and returns it as the folder has it now, when the folder says.
    fn write(&self, app: &AppHandle, path: &str, bytes: &[u8], modified_ms: i64) -> Result<Option<FileStamp>, BackendError> {
        match self {
            Self::Folder(root) => write_synced(&synced_target(root, path)?, bytes, modified_ms).map(Some),
            Self::Saf { tree_uri, folders } => {
                crate::host_sync::check_synced_path(path)?;
                saf::write(app, tree_uri, folders, path, bytes)
            }
        }
    }

    fn delete(&self, app: &AppHandle, path: &str) -> Result<(), BackendError> {
        match self {
            Self::Folder(root) => delete_synced(root, &synced_target(root, path)?),
            Self::Saf { tree_uri, .. } => {
                crate::host_sync::check_synced_path(path)?;
                saf::delete(app, tree_uri, path)
            }
        }
    }

    /// Replaces the database with the host's snapshot.
    fn write_database(&self, app: &AppHandle, bytes: &[u8]) -> Result<(), BackendError> {
        match self {
            Self::Folder(root) => {
                let notia = root.join(".notia");
                let temporary = notia.join(format!(".notia-sync-{}.tmp", uuid::Uuid::new_v4()));
                std::fs::write(&temporary, bytes).map_err(|_| storage("No se pudo guardar la copia de la base de datos."))?;
                for stale in ["notia.db-wal", "notia.db-shm"] {
                    let _ = std::fs::remove_file(notia.join(stale));
                }
                std::fs::rename(&temporary, notia.join("notia.db")).map_err(|_| {
                    let _ = std::fs::remove_file(&temporary);
                    storage("No se pudo guardar la copia de la base de datos.")
                })
            }
            Self::Saf { tree_uri, folders } => {
                saf::write(app, tree_uri, folders, DATABASE_PATH, bytes).map(|_| saf::database_replaced(app))
            }
        }
    }

    /// The copy's database, writable, when it holds the host's tables.
    fn open_database(&self, app: &AppHandle) -> Result<rusqlite::Connection, String> {
        match self {
            Self::Folder(root) => crate::database::open_copy_database_file(&root.join(".notia").join("notia.db")),
            Self::Saf { tree_uri, .. } => saf::open_database(app, tree_uri),
        }
    }

    /// Keeps what was written in the copy's database (Android writes its
    /// working copy back to the folder).
    fn save_database(&self, app: &AppHandle) -> Result<(), BackendError> {
        match self {
            Self::Folder(_) => Ok(()),
            Self::Saf { tree_uri, .. } => saf::save_database(app, tree_uri),
        }
    }

    /// The host library the copy belongs to, when marked.
    fn mark(&self, app: &AppHandle) -> Option<String> {
        let bytes = match self {
            Self::Folder(root) => std::fs::read(root.join(".notia").join("notia-copy.json")).ok()?,
            Self::Saf { tree_uri, .. } => saf::read(app, tree_uri, MARK_PATH).ok()??,
        };
        let mark: CopyMark = serde_json::from_slice(&bytes).ok()?;
        (!mark.host_library_id.is_empty()).then_some(mark.host_library_id)
    }

    fn write_mark(&self, app: &AppHandle, host_library_id: &str) -> Result<(), BackendError> {
        let text = serde_json::to_vec(&CopyMark { host_library_id: host_library_id.to_string() }).map_err(|_| copy_write_error())?;
        match self {
            Self::Folder(root) => std::fs::write(root.join(".notia").join("notia-copy.json"), text).map_err(|_| copy_write_error()),
            Self::Saf { tree_uri, folders } => saf::write(app, tree_uri, folders, MARK_PATH, &text).map(|_| ()),
        }
    }

    /// The folder changed: what shows it again reads it fresh.
    fn changed(&self, app: &AppHandle) {
        if let Self::Saf { tree_uri, .. } = self {
            saf::changed(app, tree_uri);
        }
    }
}

/// The copy's folder for this device, starting the record over when the
/// person chose another folder.
fn copy_store(app: &AppHandle, record: &mut CopyRecord, library_id: &str, library_name: &str) -> Result<CopyStore, BackendError> {
    if cfg!(target_os = "android") {
        let folder = crate::connection::settings(app)
            .copy_folder
            .ok_or_else(|| unavailable("Elegí la carpeta de la copia en Configuraciones → General."))?;
        if record.folder_uri.as_deref() != Some(folder.uri.as_str()) {
            *record = CopyRecord { folder_uri: Some(folder.uri.clone()), ..CopyRecord::default() };
        }
        return Ok(CopyStore::Saf { tree_uri: folder.uri, folders: Mutex::default() });
    }
    let folder = match &record.folder {
        Some(folder) => folder.clone(),
        None => directory(app)?.join(folder_name(library_id, "biblioteca")).join(folder_name(library_name, "Biblioteca")),
    };
    std::fs::create_dir_all(folder.join(".notia")).map_err(|_| storage("No se pudo crear la carpeta de la copia."))?;
    let root = std::fs::canonicalize(&folder).map_err(|_| storage("No se pudo abrir la carpeta de la copia."))?;
    record.folder = Some(folder);
    Ok(CopyStore::Folder(root))
}

// ---------- Sync ----------

/// Brings the copy and the host's library in line (see the module).
pub(crate) async fn sync(app: &AppHandle) -> Result<CopyStatus, BackendError> {
    if !keeps_copy(app) {
        return Err(unavailable("Este equipo no guarda una copia de la biblioteca del host."));
    }
    let state = state(app).ok_or_else(|| unavailable("La copia no está disponible."))?;
    let _guard = state.sync.lock().await;
    let (library_id, library_name) = crate::host_client::served_library(app)
        .ok_or_else(|| unavailable("Todavía no se sabe qué biblioteca comparte el host."))?;
    let result = sync_library(app, &state, &library_id, &library_name).await;
    state.transferring.store(false, Ordering::SeqCst);
    if let Ok(mut progress) = state.progress.lock() {
        *progress = None;
    }
    let status = match &result {
        Ok(status) => status.clone(),
        Err(error) => CopyStatus { at_ms: now_ms(), error: Some(error.message.clone()), ..CopyStatus::default() },
    };
    if let Ok(mut last) = state.last.lock() {
        *last = Some(status.clone());
    }
    let _ = app.emit(COPY_EVENT, &status);
    announce_changes(app, &status);
    crate::client_status::refresh(app);
    result
}

/// The window reads again what a sync changed in the copy it works on:
/// the files of the explorer and the modules that show the database.
fn announce_changes(app: &AppHandle, status: &CopyStatus) {
    if !is_open(app) {
        return;
    }
    if status.downloaded + status.deleted > 0 {
        let _ = app.emit(crate::filesystem::watch::LIBRARY_TREE_CHANGED_EVENT, serde_json::Value::Null);
    }
    if status.rows_downloaded > 0 {
        for event in [
            crate::finance::FINANCE_DATA_CHANGED_EVENT,
            crate::agenda_tools::AGENDA_DATA_CHANGED_EVENT,
            crate::routine::ROUTINE_DATA_CHANGED_EVENT,
        ] {
            let _ = app.emit(event, ());
        }
    }
}

/// Whether the window works on the copy now.
pub(crate) fn is_open(app: &AppHandle) -> bool {
    state(app).is_some_and(|state| state.open.load(Ordering::SeqCst))
}

/// Starts a sync on its own thread, unless one already runs: the window
/// and the monitor of the host never wait for it.
pub(crate) fn sync_in_background(app: &AppHandle) {
    if !keeps_copy(app) {
        return;
    }
    let Some(state) = state(app) else {
        return;
    };
    if state.background.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("notia-copy-sync".into()).spawn(move || {
        if let Err(error) = crate::host::async_runtime::block_on(sync(&app)) {
            log::warn!("[notia:copy] the copy did not sync: {}", error.message);
        }
        if let Some(state) = self::state(&app) {
            state.background.store(false, Ordering::SeqCst);
        }
    });
    if spawned.is_err() {
        state.background.store(false, Ordering::SeqCst);
        log::error!("[notia:copy] the sync of the copy did not start");
    }
}

async fn sync_library(app: &AppHandle, state: &MirrorState, library_id: &str, library_name: &str) -> Result<CopyStatus, BackendError> {
    // A computer tells the host how much it processes, for deciding which
    // one runs the library's services (`device_role`); Android never does.
    let device = cfg!(not(any(target_os = "android", target_os = "ios"))).then(|| crate::device_role::local_power(app));
    let payload = json!({ "payload": { "libraryId": library_id, "device": device } });
    let manifest: SyncManifest = crate::host_client::call_host(app, "host_sync_manifest", payload).await?;
    crate::device_role::note_synced(app);
    let record_file = record_path(app, library_id)?;
    let mut record: CopyRecord = read_json(&record_file);
    // A library the host gave a new id goes on from its copy (see
    // `adopt_previous_record`).
    let adopted = if record.database_ms.is_none() && record.files.is_empty() {
        adopt_previous_record(app, library_id, library_name, manifest.database_id.as_deref(), &mut record, &record_file)?
    } else {
        None
    };
    let store = copy_store(app, &mut record, library_id, library_name)?;
    let mut mark = store.mark(app);
    if mark.as_deref().is_some_and(|mark| mark != library_id) {
        if mark != adopted {
            return Err(unavailable("La carpeta de la copia tiene la copia de otra biblioteca: elegí otra en Configuraciones → General."));
        }
        store.write_mark(app, library_id)?;
        mark = Some(library_id.to_string());
    }

    // The copy is listed when this device may have changed it (desktop
    // always; Android after a write of this device since the last listing).
    let writes = saf::local_writes();
    let scan = cfg!(not(target_os = "android")) || state.listed_at.lock().map_or(true, |listed| *listed != Some(writes));
    let local: BTreeMap<String, FileStamp> = if scan {
        store.list(app)?
    } else {
        record.files.iter().map(|(path, base)| (path.clone(), base.local)).collect()
    };
    let remote: BTreeMap<String, FileStamp> = manifest.files.iter().map(|file| (file.path.clone(), file.stamp)).collect();
    let paths: BTreeSet<String> = record.files.keys().chain(local.keys()).chain(remote.keys()).cloned().collect();

    // What changed on this device travels first: the other devices see it
    // as soon as possible. Then the database (the copy opens only with it:
    // the sign-in reads the Owner from it) and last what the host changed.
    let mut plans: Vec<(String, SyncAction)> = paths
        .into_iter()
        .map(|path| {
            let action = plan(record.files.get(&path), local.get(&path), remote.get(&path));
            (path, action)
        })
        .filter(|(_, action)| *action != SyncAction::Keep)
        .collect();
    plans.sort_by_key(|(_, action)| !is_outgoing(*action));
    let host_database = manifest.database.is_some();
    // A copy without the database, or from before the rows travelled,
    // takes a whole snapshot.
    let snapshot_due = host_database && (record.database_ms.is_none() || record.database_id.is_none());
    let first_database = snapshot_due && record.database_ms.is_none();
    if !plans.is_empty() || snapshot_due {
        state.transferring.store(true, Ordering::SeqCst);
        if let Ok(mut progress) = state.progress.lock() {
            *progress = Some((0, plans.len()));
        }
        crate::client_status::refresh(app);
    }
    let _running = (plans.len() >= LONG_SYNC_FILES || first_database).then(|| KeepRunning::begin(app));
    let (outgoing, incoming) = plans.split_at(plans.partition_point(|(_, action)| is_outgoing(*action)));

    let mut pass = Pass {
        app,
        state,
        store: &store,
        library_id,
        host_root: &manifest.root,
        local: &local,
        remote: &remote,
        record: &mut record,
        record_file: &record_file,
        status: CopyStatus::default(),
        downloaded: Vec::new(),
        failure: None,
        done: 0,
        total: plans.len(),
        saved_at: std::time::Instant::now(),
    };
    pass.run(outgoing).await;
    if pass.failure.is_none() && host_database {
        match sync_database(app, &store, library_id, pass.record, &mut pass.status).await {
            // With the database recorded, the copy opens even if the rest
            // of this sync is cut short.
            Ok(()) => pass.save(),
            Err(error) if connection_lost(&error) => pass.failure = Some(error),
            // The files still travel; the database is tried again next time.
            Err(error) => {
                log::error!("[notia:copy] the database of the copy did not sync: {}", error.message);
                pass.status.error = Some(error.message);
            }
        }
    }
    pass.run(incoming).await;
    let Pass { mut status, downloaded, mut failure, .. } = pass;

    // What the copy's folder says about the files just written.
    if !downloaded.is_empty() {
        if let Ok(after) = store.list(app) {
            for path in &downloaded {
                if let (Some(stamp), Some(base)) = (after.get(path), record.files.get_mut(path)) {
                    base.local = *stamp;
                }
            }
        }
    }

    if failure.is_none() && mark.is_none() {
        if let Err(error) = store.write_mark(app, library_id) {
            failure = Some(error);
        }
    }
    if status.downloaded + status.deleted > 0 {
        store.changed(app);
    }
    // What was done stays recorded even when the connection dropped.
    write_json(&record_file, &record)?;
    if let Some(error) = failure {
        return Err(error);
    }
    let current = CurrentCopy {
        host_address: crate::connection::settings(app).host_address,
        library_id: library_id.to_string(),
        library_name: library_name.to_string(),
    };
    write_json(&directory(app)?.join(CURRENT_FILE), &current)?;
    if scan {
        if let Ok(mut listed) = state.listed_at.lock() {
            *listed = Some(writes);
        }
    }
    status.at_ms = now_ms();
    Ok(status)
}

/// When the host gives its library a new id (its catalog was made again:
/// the library keeps its folder and its database), the copy made under the
/// old id goes on under the new one, with what it had synced and what it
/// changed meanwhile, instead of being refused as another library's copy or
/// copied again from scratch. `current.json` says which library this
/// device copied last, and the record of that copy says which host
/// database it follows: the same database is the same library. A copy from
/// before the row log has no database id; it is the same library when the
/// host and the library's name are the same. Returns the old id.
fn adopt_previous_record(
    app: &AppHandle,
    library_id: &str,
    library_name: &str,
    host_database_id: Option<&str>,
    record: &mut CopyRecord,
    record_file: &Path,
) -> Result<Option<String>, BackendError> {
    let directory = directory(app)?;
    let current: CurrentCopy = read_json(&directory.join(CURRENT_FILE));
    if current.library_id.is_empty() || current.library_id == library_id {
        return Ok(None);
    }
    let previous_file = record_path(app, &current.library_id)?;
    let previous: CopyRecord = read_json(&previous_file);
    let host_address = crate::connection::settings(app).host_address;
    let same_library = match (&previous.database_id, host_database_id) {
        (Some(copied), Some(host)) => copied == host,
        (None, _) => previous.database_ms.is_some() && current.host_address == host_address && current.library_name == library_name,
        (Some(_), None) => false,
    };
    if !same_library {
        return Ok(None);
    }
    *record = previous;
    write_json(record_file, record)?;
    let _ = std::fs::remove_file(&previous_file);
    // The copy opens under the new id from now on.
    let next = CurrentCopy { host_address, library_id: library_id.to_string(), library_name: current.library_name };
    write_json(&directory.join(CURRENT_FILE), &next)?;
    log::warn!("[notia:copy] the copy goes on under the new id the host gave the library");
    Ok(Some(current.library_id))
}

/// Whether `error` means the host stopped answering (the sync stops) rather
/// than one file failing (it is skipped).
fn connection_lost(error: &BackendError) -> bool {
    matches!(error.code, BackendErrorCode::ProviderUnavailable | BackendErrorCode::Unauthorized | BackendErrorCode::Forbidden)
}

/// Whether `action` takes a change of this device to the host.
fn is_outgoing(action: SyncAction) -> bool {
    matches!(action, SyncAction::Upload | SyncAction::DeleteRemote)
}

/// One run over the files of a sync, recording what travelled.
struct Pass<'a> {
    app: &'a AppHandle,
    state: &'a MirrorState,
    store: &'a CopyStore,
    library_id: &'a str,
    host_root: &'a str,
    local: &'a BTreeMap<String, FileStamp>,
    remote: &'a BTreeMap<String, FileStamp>,
    record: &'a mut CopyRecord,
    record_file: &'a Path,
    status: CopyStatus,
    /// Files written in the copy whose folder did not say how it keeps
    /// them: their stamps are read by listing the folder afterwards.
    downloaded: Vec<String>,
    /// Why the sync stopped (the host stopped answering).
    failure: Option<BackendError>,
    done: usize,
    total: usize,
    saved_at: std::time::Instant,
}

impl Pass<'_> {
    async fn run(&mut self, plans: &[(String, SyncAction)]) {
        for (path, action) in plans {
            if self.failure.is_some() {
                return;
            }
            let (local, remote) = (self.local.get(path), self.remote.get(path));
            let applied = apply(self.app, self.store, self.library_id, self.host_root, path, *action, local, remote, &mut self.status).await;
            match applied {
                Ok(Outcome::Written { base, stamped }) => {
                    if !stamped {
                        self.downloaded.push(path.clone());
                    }
                    self.record.files.insert(path.clone(), base);
                }
                Ok(Outcome::Base(next)) => {
                    self.record.files.insert(path.clone(), next);
                }
                Ok(Outcome::Drop) => {
                    self.record.files.remove(path);
                }
                Ok(Outcome::Unchanged) => {}
                // Without the host the sync stops; a file that cannot travel
                // is skipped, and the next sync tries it again.
                Err(error) if connection_lost(&error) => self.failure = Some(error),
                Err(error) => {
                    log::error!("[notia:copy] a file of the copy did not sync: {}", error.message);
                    self.status.skipped.push(path.clone());
                }
            }
            self.done += 1;
            if self.saved_at.elapsed() >= SAVE_EVERY {
                self.save();
            }
        }
    }

    /// Saves what travelled so far and shows how far the sync went.
    fn save(&mut self) {
        if let Err(error) = write_json(self.record_file, &*self.record) {
            log::error!("[notia:copy] the progress of the copy was not saved: {}", error.message);
        }
        if let Ok(mut progress) = self.state.progress.lock() {
            *progress = Some((self.done, self.total));
        }
        crate::client_status::refresh(self.app);
        self.saved_at = std::time::Instant::now();
    }
}

/// What one action leaves in the record of the file.
enum Outcome {
    Base(SyncBase),
    /// A file written in the copy; `stamped` when the folder said how it
    /// keeps it (otherwise it is read by listing the folder afterwards).
    Written { base: SyncBase, stamped: bool },
    Drop,
    /// Nothing changed, or the file did not travel: the next sync tries again.
    Unchanged,
}

/// Keeps an Android process running while a long sync moves files, even
/// with the window hidden (a foreground service of type `dataSync`).
struct KeepRunning<'a> {
    app: &'a AppHandle,
    started: bool,
}

impl<'a> KeepRunning<'a> {
    fn begin(app: &'a AppHandle) -> Self {
        let started = app
            .try_state::<crate::mobile_continuity::ContinuityState>()
            .is_some_and(|state| crate::mobile_continuity::begin_android_work(state.inner(), Some("dataSync")));
        Self { app, started }
    }
}

impl Drop for KeepRunning<'_> {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        if let Some(state) = self.app.try_state::<crate::mobile_continuity::ContinuityState>() {
            crate::mobile_continuity::end_android_work(state.inner());
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply(
    app: &AppHandle,
    store: &CopyStore,
    library_id: &str,
    host_root: &str,
    path: &str,
    action: SyncAction,
    local: Option<&FileStamp>,
    remote: Option<&FileStamp>,
    status: &mut CopyStatus,
) -> Result<Outcome, BackendError> {
    let base = |remote: FileStamp, local: FileStamp| Outcome::Base(SyncBase { remote, local });
    match (action, local, remote) {
        (SyncAction::Adopt, Some(local), Some(remote)) => Ok(base(*remote, *local)),
        (SyncAction::Download, _, Some(remote)) => {
            let Some((_, bytes)) = crate::host_client::fetch_file(app, &format!("{host_root}/{path}")).await else {
                status.skipped.push(path.to_string());
                return Ok(Outcome::Unchanged);
            };
            let written = store.write(app, path, &bytes, remote.modified_ms)?;
            status.downloaded += 1;
            Ok(Outcome::Written {
                base: SyncBase { remote: *remote, local: written.unwrap_or(*remote) },
                stamped: written.is_some(),
            })
        }
        (SyncAction::Upload, Some(local), _) => {
            if local.size > MAX_SYNC_UPLOAD_BYTES {
                status.skipped.push(path.to_string());
                return Ok(Outcome::Unchanged);
            }
            let bytes = store.read(app, path)?;
            let payload = json!({ "payload": {
                "libraryId": library_id,
                "path": path,
                "contentBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
                "modifiedMs": local.modified_ms,
            }});
            let written: SyncFile = crate::host_client::call_host(app, "host_sync_write", payload).await?;
            status.uploaded += 1;
            Ok(base(written.stamp, *local))
        }
        (SyncAction::DeleteLocal, _, _) => {
            store.delete(app, path)?;
            status.deleted += 1;
            Ok(Outcome::Drop)
        }
        (SyncAction::DeleteRemote, _, _) => {
            let payload = json!({ "payload": { "libraryId": library_id, "path": path } });
            crate::host_client::call_host::<serde_json::Value>(app, "host_sync_delete", payload).await?;
            status.deleted += 1;
            Ok(Outcome::Drop)
        }
        (SyncAction::Forget, _, _) => Ok(Outcome::Drop),
        _ => Ok(Outcome::Unchanged),
    }
}

// ---------- The database ----------

fn copy_database_error() -> BackendError {
    storage("No se pudo leer o escribir la base de datos de la copia.")
}

/// Brings the copy's database in line with the host's (`row_sync`): what
/// this device changed goes first, then what the host changed after the
/// last change the copy took. A copy without the database, or following
/// another one, takes a whole snapshot after sending its own changes.
async fn sync_database(
    app: &AppHandle,
    store: &CopyStore,
    library_id: &str,
    record: &mut CopyRecord,
    status: &mut CopyStatus,
) -> Result<(), BackendError> {
    if record.database_ms.is_some() && record.database_id.is_some() {
        match store.open_database(app) {
            Ok(connection) => {
                let connection = Mutex::new(connection);
                let mut changed = false;
                let exchanged = exchange_rows(app, &connection, library_id, record, status, &mut changed).await;
                drop(connection);
                // Android keeps the database in the copy's folder.
                let saved = if changed { store.save_database(app) } else { Ok(()) };
                let next = exchanged?;
                saved?;
                // The cursor moves once the rows are saved.
                if let Some(seq) = next {
                    record.database_seq = seq;
                    return Ok(());
                }
            }
            // A database that never arrived is copied whole; any other
            // failure keeps the copy as it is.
            Err(message) if message == crate::database::COPY_WITHOUT_DATABASE => {}
            Err(message) => {
                log::error!("[notia:copy] the database of the copy did not open: {message}");
                return Err(copy_database_error());
            }
        }
    }
    copy_database(app, store, library_id, record).await
}

/// Sends the rows this device changed, then takes the host's. Returns the
/// last change of the host the copy now has, or `None` when the copy must
/// take a snapshot (the host's database is another one). `changed` says
/// whether the copy's database was written.
async fn exchange_rows(
    app: &AppHandle,
    connection: &Mutex<rusqlite::Connection>,
    library_id: &str,
    record: &CopyRecord,
    status: &mut CopyStatus,
    changed: &mut bool,
) -> Result<Option<i64>, BackendError> {
    // The lock is never held while waiting for the host.
    let rows = |work: &mut dyn FnMut(&rusqlite::Connection) -> rusqlite::Result<()>| -> Result<(), BackendError> {
        let connection = connection.lock().map_err(|_| copy_database_error())?;
        work(&connection).map_err(|_| copy_database_error())
    };
    loop {
        let mut pending = Vec::new();
        rows(&mut |connection| {
            pending = crate::row_sync::pending_changes(connection, MAX_ROW_CHANGES)?;
            Ok(())
        })?;
        if pending.is_empty() {
            break;
        }
        let payload = json!({ "payload": { "libraryId": library_id, "changes": &pending } });
        let report: ApplyReport = crate::host_client::call_host(app, "host_sync_database_push", payload).await?;
        rows(&mut |connection| crate::row_sync::mark_sent(connection, &pending))?;
        *changed = true;
        status.rows_uploaded += report.applied;
    }
    let mut since = record.database_seq;
    loop {
        let payload = json!({ "payload": { "libraryId": library_id, "since": since, "databaseId": record.database_id } });
        let reply: DatabaseChanges = crate::host_client::call_host(app, "host_sync_database_changes", payload).await?;
        if reply.reset {
            return Ok(None);
        }
        if !reply.batch.changes.is_empty() {
            let mut report = ApplyReport::default();
            rows(&mut |connection| {
                report = crate::row_sync::apply_changes(connection, &reply.batch.changes)?;
                Ok(())
            })?;
            *changed |= report.applied > 0;
            status.rows_downloaded += report.applied;
        }
        since = reply.batch.last_seq;
        if !reply.batch.more {
            return Ok(Some(since));
        }
    }
}

/// Replaces the copy's database with a snapshot of the host's.
async fn copy_database(app: &AppHandle, store: &CopyStore, library_id: &str, record: &mut CopyRecord) -> Result<(), BackendError> {
    let payload = json!({ "payload": { "libraryId": library_id } });
    let snapshot: Option<DatabaseSnapshot> = crate::host_client::call_host(app, "host_sync_database", payload).await?;
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(snapshot.content_base64.as_bytes())
        .map_err(|_| storage("La copia de la base de datos no es válida."))?;
    store.write_database(app, &bytes)?;
    record.database_ms = Some(snapshot.modified_ms);
    record.database_id = snapshot.database_id;
    record.database_seq = snapshot.seq;
    Ok(())
}

// ---------- The SAF folder of the copy (Android) ----------

#[cfg(target_os = "android")]
mod saf {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::backend::mirror_sync::{is_synced_path, FileStamp};
    use crate::backend::{BackendError, BackendErrorCode, LogicalPathDto};
    use crate::host::{AppHandle, Manager};
    use crate::mobile_directory_picker::{self as picker, AndroidDirectoryPickerState};

    fn failed(message: impl Into<String>) -> BackendError {
        BackendError::new(BackendErrorCode::Storage, message, true)
    }

    fn picker_state(app: &AppHandle) -> &'static AndroidDirectoryPickerState {
        app.state::<AndroidDirectoryPickerState>().inner()
    }

    fn segments(path: &str) -> Result<Vec<String>, BackendError> {
        let logical = LogicalPathDto::new(path)?;
        Ok(logical.as_str().split('/').map(str::to_owned).collect())
    }

    /// The document URI of a file of the copy, when it exists.
    fn document(app: &AppHandle, tree_uri: &str, path: &str) -> Result<Option<String>, BackendError> {
        LogicalPathDto::new(path)?;
        let lookup = format!("{}/{path}", tree_uri.trim_end_matches('/'));
        Ok(crate::filesystem::android_saf::resolve_document_uri(picker_state(app), &lookup, Some(tree_uri)))
    }

    pub(super) fn list(app: &AppHandle, tree_uri: &str) -> Result<BTreeMap<String, FileStamp>, BackendError> {
        let entries = picker::read_android_flat_entries(picker_state(app), tree_uri)
            .map_err(|_| failed("No se pudo leer la carpeta de la copia; volvé a elegirla en Configuraciones → General."))?;
        let prefix = format!("{}/", tree_uri.trim_end_matches('/'));
        Ok(entries
            .into_iter()
            .filter(|entry| entry.node_type == "file")
            .filter_map(|entry| {
                let path = entry.path.strip_prefix(&prefix)?.to_string();
                (is_synced_path(&path) && LogicalPathDto::new(&path).is_ok()).then(|| {
                    let stamp = FileStamp { size: entry.size.unwrap_or_default(), modified_ms: entry.last_modified.unwrap_or_default() };
                    (path, stamp)
                })
            })
            .collect())
    }

    pub(super) fn read(app: &AppHandle, tree_uri: &str, path: &str) -> Result<Option<Vec<u8>>, BackendError> {
        let Some(uri) = document(app, tree_uri, path)? else {
            return Ok(None);
        };
        picker::read_android_content_bytes(picker_state(app), &uri)
            .map(|(_, bytes)| Some(bytes))
            .map_err(|_| failed("No se pudo leer un archivo de la copia."))
    }

    /// Creates the file (and its folders) when missing and writes it.
    /// Returns the file as the folder keeps it, when the provider says.
    pub(super) fn write(
        app: &AppHandle,
        tree_uri: &str,
        folders: &Mutex<BTreeMap<String, String>>,
        path: &str,
        bytes: &[u8],
    ) -> Result<Option<FileStamp>, BackendError> {
        let state = picker_state(app);
        let segments = segments(path)?;
        let create_failed = |_| failed("No se pudo crear un archivo en la copia.");
        let uri = match segments.split_last() {
            Some((name, parents)) if !parents.is_empty() => {
                let folder = parents.join("/");
                let known = folders.lock().ok().and_then(|folders| folders.get(&folder).cloned());
                let parent = match known {
                    Some(uri) => uri,
                    None => {
                        let uri = picker::create_android_path_entry(state, tree_uri, parents, "folder", None).map_err(create_failed)?;
                        if let Ok(mut folders) = folders.lock() {
                            folders.insert(folder, uri.clone());
                        }
                        uri
                    }
                };
                picker::create_android_tree_entry(state, &parent, name, "file", None).map_err(create_failed)?
            }
            _ => picker::create_android_path_entry(state, tree_uri, &segments, "file", None).map_err(create_failed)?,
        };
        let stamp = picker::write_android_content_bytes_stamped(state, &uri, bytes)
            .map_err(|_| failed("No se pudo guardar un archivo en la copia."))?;
        picker::put_android_path_lru(state, format!("{}/{path}", tree_uri.trim_end_matches('/')), uri);
        Ok(stamp.map(|(size, modified_ms)| FileStamp { size, modified_ms }))
    }

    pub(super) fn delete(app: &AppHandle, tree_uri: &str, path: &str) -> Result<(), BackendError> {
        let Some(uri) = document(app, tree_uri, path)? else {
            return Ok(());
        };
        picker::delete_android_tree_entry(picker_state(app), &uri).map_err(|_| failed("No se pudo borrar un archivo de la copia."))?;
        picker::invalidate_android_path_lru(picker_state(app), &format!("{}/{path}", tree_uri.trim_end_matches('/')));
        Ok(())
    }

    /// The temporary copies of SQLite are made again from the new snapshot.
    pub(super) fn database_replaced(app: &AppHandle) {
        crate::database::discard_mobile_database_copies(app);
    }

    pub(super) fn open_database(app: &AppHandle, tree_uri: &str) -> Result<rusqlite::Connection, String> {
        crate::database::open_mobile_copy_database(app, tree_uri)
    }

    pub(super) fn save_database(app: &AppHandle, tree_uri: &str) -> Result<(), BackendError> {
        crate::database::sync_mobile_library_connection(app, tree_uri).map_err(failed)
    }

    pub(super) fn changed(app: &AppHandle, tree_uri: &str) {
        picker::invalidate_tree_cache(picker_state(app), tree_uri);
        picker::mark_tree_mutated(picker_state(app), tree_uri);
    }

    pub(super) fn local_writes() -> u64 {
        picker::library_writes()
    }

    /// The person picks the folder of the copy. It must be empty, or an
    /// earlier copy: other files would travel to the host as new ones.
    pub(super) fn pick_folder(app: &AppHandle) -> Result<crate::backend::connection::CopyFolder, BackendError> {
        let picked = picker::pick_android_directory_tree(app.state(), app.state(), None).map_err(failed)?;
        let uri = picked.uri.ok_or_else(|| failed("El selector no devolvió una carpeta."))?;
        let entries = picker::read_android_flat_entries(picker_state(app), &uri)
            .map_err(|_| failed("No se pudo leer la carpeta elegida."))?;
        let prefix = format!("{}/", uri.trim_end_matches('/'));
        let marked = entries.iter().any(|entry| entry.path.strip_prefix(&prefix) == Some(super::MARK_PATH));
        let has_files = entries.iter().any(|entry| {
            let path = entry.path.strip_prefix(&prefix).unwrap_or(&entry.path);
            path != ".notia" && !path.starts_with(".notia/")
        });
        if has_files && !marked {
            return Err(BackendError::invalid_input(
                "Elegí una carpeta vacía para la copia: lo que tenga se subiría al host como archivos nuevos.",
            ));
        }
        let name = crate::backend::library_tree::library_display_name(&uri);
        Ok(crate::backend::connection::CopyFolder { uri, name })
    }
}

#[cfg(not(target_os = "android"))]
mod saf {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::backend::mirror_sync::FileStamp;
    use crate::backend::BackendError;
    use crate::host::AppHandle;

    fn unavailable() -> BackendError {
        super::unavailable("La carpeta SAF de la copia solo existe en Android.")
    }

    pub(super) fn list(_app: &AppHandle, _tree_uri: &str) -> Result<BTreeMap<String, FileStamp>, BackendError> {
        Err(unavailable())
    }

    pub(super) fn read(_app: &AppHandle, _tree_uri: &str, _path: &str) -> Result<Option<Vec<u8>>, BackendError> {
        Err(unavailable())
    }

    pub(super) fn write(
        _app: &AppHandle,
        _tree_uri: &str,
        _folders: &Mutex<BTreeMap<String, String>>,
        _path: &str,
        _bytes: &[u8],
    ) -> Result<Option<FileStamp>, BackendError> {
        Err(unavailable())
    }

    pub(super) fn delete(_app: &AppHandle, _tree_uri: &str, _path: &str) -> Result<(), BackendError> {
        Err(unavailable())
    }

    pub(super) fn database_replaced(_app: &AppHandle) {}

    pub(super) fn open_database(_app: &AppHandle, _tree_uri: &str) -> Result<rusqlite::Connection, String> {
        Err(unavailable().message)
    }

    pub(super) fn save_database(_app: &AppHandle, _tree_uri: &str) -> Result<(), BackendError> {
        Err(unavailable())
    }

    pub(super) fn changed(_app: &AppHandle, _tree_uri: &str) {}

    /// Windows, Linux and macOS list the copy on every sync.
    pub(super) fn local_writes() -> u64 {
        0
    }
}

/// Android: the person picks the folder of the copy (Configuraciones →
/// General). Windows and Linux keep it in the app data folder.
pub(crate) async fn pick_copy_folder(app: AppHandle) -> Result<(), BackendError> {
    #[cfg(target_os = "android")]
    {
        let picker_app = app.clone();
        let folder = crate::host::async_runtime::spawn_blocking(move || saf::pick_folder(&picker_app))
            .await
            .map_err(|_| storage("No se pudo abrir el selector de carpetas."))??;
        let mut settings = crate::connection::settings(&app);
        settings.copy_folder = Some(folder);
        crate::connection::store(&app, &settings)?;
        // The new folder is listed on the next sync.
        if let Some(state) = state(&app) {
            if let Ok(mut listed) = state.listed_at.lock() {
                *listed = None;
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(unavailable("En este equipo la copia se guarda sola en la carpeta de datos de Notia."))
    }
}

// ---------- The copy as the library of this device ----------

/// Opens the copy as the library of this device: the window works on it,
/// and it syncs with the host in the background. Returns its id.
pub(crate) fn open_copy(app: &AppHandle) -> Result<String, BackendError> {
    if !keeps_copy(app) {
        return Err(unavailable("Este equipo no guarda una copia de la biblioteca del host."));
    }
    let never_synced = || {
        BackendError::new(
            BackendErrorCode::NotFound,
            "Todavía no hay una copia de la biblioteca en este equipo: conectate al host al menos una vez.",
            true,
        )
    };
    let current: CurrentCopy = read_json(&directory(app)?.join(CURRENT_FILE));
    if current.library_id.is_empty() || current.host_address != crate::connection::settings(app).host_address {
        return Err(never_synced());
    }
    let record_file = record_path(app, &current.library_id)?;
    let mut record: CopyRecord = read_json(&record_file);
    if record.database_ms.is_none() {
        return Err(BackendError::new(
            BackendErrorCode::NotFound,
            "La copia de este equipo todavía no tiene la base de datos de la biblioteca: conectate al host para completarla.",
            true,
        ));
    }
    let (library, location) = open_copy_library(app, &record).ok_or_else(never_synced)??;
    let previous = crate::library_catalog::select_library(app, Some(&library.id))?;
    if previous.as_deref() != Some(library.id.as_str()) && record.local_library_id.as_deref() != previous.as_deref() {
        record.previous_selection = previous;
    }
    record.local_library_id = Some(library.id.clone());
    write_json(&record_file, &record)?;
    if let Ok(mut copy) = OPEN_COPY.lock() {
        *copy = Some(location);
    }
    if let Some(state) = state(app) {
        state.open.store(true, Ordering::SeqCst);
    }
    crate::client_status::refresh(app);
    Ok(library.id)
}

/// The copy as a library of this device (added to the catalog), or `None`
/// when this device has not synced it yet.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn open_copy_library(
    app: &AppHandle,
    record: &CopyRecord,
) -> Option<Result<(crate::library_catalog::CatalogLibrary, CopyLocation), BackendError>> {
    let folder = record.folder.clone().filter(|folder| folder.is_dir())?;
    Some(crate::library_catalog::add_desktop_library(app, &folder).map(|library| {
        (library, CopyLocation::Folder(std::fs::canonicalize(&folder).unwrap_or(folder)))
    }))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn open_copy_library(
    app: &AppHandle,
    record: &CopyRecord,
) -> Option<Result<(crate::library_catalog::CatalogLibrary, CopyLocation), BackendError>> {
    let folder = crate::connection::settings(app).copy_folder?;
    if record.folder_uri.as_deref() != Some(folder.uri.as_str()) || record.database_ms.is_none() && record.files.is_empty() {
        return None;
    }
    Some(crate::library_catalog::add_android_library(app, &folder.uri).map(|library| (library, CopyLocation::Saf(folder.uri))))
}

/// This device goes back to Host mode: the window opens the library it
/// had before the copy. The copy stays for a later return to Client.
pub(crate) fn close_copy(app: &AppHandle) -> Result<(), BackendError> {
    if let Some(state) = state(app) {
        state.open.store(false, Ordering::SeqCst);
        if let Ok(mut listed) = state.listed_at.lock() {
            *listed = None;
        }
    }
    if let Ok(mut copy) = OPEN_COPY.lock() {
        *copy = None;
    }
    let current: CurrentCopy = read_json(&directory(app)?.join(CURRENT_FILE));
    if current.library_id.is_empty() {
        return Ok(());
    }
    let record_file = record_path(app, &current.library_id)?;
    let mut record: CopyRecord = read_json(&record_file);
    if let Some(previous) = record.previous_selection.take() {
        let _ = crate::library_catalog::select_library(app, Some(&previous));
        write_json(&record_file, &record)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Mi: Biblioteca/2026", "x"), "Mi Biblioteca2026");
        assert_eq!(folder_name("..", "Biblioteca"), "Biblioteca");
        assert_eq!(folder_name("  ", "Biblioteca"), "Biblioteca");
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn the_copy_follows_the_host_and_keeps_the_last_modified_file() {
        use crate::backend::connection::{ConnectionSettings, RunMode};
        use crate::host::{AppPaths, HostPorts};

        let root = std::env::temp_dir().join(format!("notia-copy-{}", uuid::Uuid::new_v4()));
        let (host, port, stop, library_id) = crate::host_client::test_host::start(&root);
        let library = root.join("Biblioteca");
        std::fs::create_dir_all(library.join("Notas")).expect("notes");
        let write = |path: &Path, text: &str, modified_ms: i64| {
            std::fs::write(path, text).expect("write");
            let file = std::fs::OpenOptions::new().write(true).open(path).expect("open");
            file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(modified_ms as u64)).expect("time");
        };
        let base_ms = 1_750_000_000_000_i64;
        write(&library.join("Notas/a.md"), "uno", base_ms);
        write(&library.join("b.md"), "dos", base_ms);
        // The handwriting of a note is part of the library.
        std::fs::create_dir_all(library.join(".notia/ink/Notas")).expect("ink folder");
        write(&library.join(".notia/ink/Notas/a.md.json"), "{\"strokes\":[]}", base_ms);

        let client = crate::create_app(AppPaths::new(Some(root.join("client")), None), HostPorts::default());
        let settings = ConnectionSettings { mode: RunMode::Client, host_address: format!("127.0.0.1:{port}"), ..ConnectionSettings::default() };
        crate::connection::store(&client, &settings).expect("settings");

        crate::host::async_runtime::block_on(async {
            crate::host_client::auth_login(&client, &library_id, "owner", "contraseña-1", false, false).await.expect("login");
            let first = sync(&client).await.expect("first sync");
            assert!(first.downloaded >= 2, "{first:?}");
        });
        let copy = directory(&client).expect("directory").join(folder_name(&library_id, "x")).join("Biblioteca");
        assert_eq!(std::fs::read_to_string(copy.join("Notas/a.md")).expect("a"), "uno");
        assert_eq!(std::fs::read_to_string(copy.join(".notia/ink/Notas/a.md.json")).expect("ink"), "{\"strokes\":[]}");
        assert!(copy.join(".notia/notia.db").is_file(), "the database snapshot is kept");

        // A first copy cut short: the record lost the files and one of them
        // was written halfway (newer and shorter). The host's comes back and
        // nothing goes up.
        let record_file = record_path(&client, &library_id).expect("record");
        let mut record: CopyRecord = read_json(&record_file);
        record.files.clear();
        write_json(&record_file, &record).expect("cut short");
        write(&copy.join("Notas/a.md"), "u", base_ms + 5_000);
        crate::host::async_runtime::block_on(async {
            let resumed = sync(&client).await.expect("resumed");
            assert_eq!((resumed.uploaded, resumed.downloaded), (0, 1), "{resumed:?}");
        });
        assert_eq!(std::fs::read_to_string(copy.join("Notas/a.md")).expect("a again"), "uno");
        assert_eq!(std::fs::read_to_string(library.join("Notas/a.md")).expect("host a"), "uno");

        // Offline: the copy edits a.md and adds c.md; the host edits b.md.
        write(&copy.join("Notas/a.md"), "uno en la copia", base_ms + 10_000);
        write(&copy.join("c.md"), "nuevo", base_ms + 10_000);
        write(&library.join("b.md"), "dos en el host", base_ms + 20_000);
        // Offline, the copy's configuration changes a setting and turns
        // Telegram on (sealed with the library's key, as the copy does).
        let unlocked = crate::config_vault::unlocked(&host, &library_id).expect("unlocked host");
        let offline_config = serde_json::json!({
            "version": 1,
            "panelDesplegable": { "refreshIntervalMs": 60_000 },
            "telegram": { "enabled": true, "botToken": "1:abc" },
        });
        let sealed = crate::config_crypto::seal_config(&unlocked.key, &unlocked.wrapped, &offline_config.to_string()).expect("seal");
        let config_path = copy.join(".notia/notiaConfig.json");
        write(&config_path, &crate::backend::config_envelope::serialize_envelope(&sealed).expect("envelope"), base_ms + 10_000);
        crate::host::async_runtime::block_on(async {
            let status = sync(&client).await.expect("reconcile");
            assert_eq!((status.uploaded, status.downloaded), (3, 1), "{status:?}");
        });
        let host_config = crate::library_config::read_library_config(&host, &library_id).expect("read").expect("config");
        assert_eq!(host_config["panelDesplegable"]["refreshIntervalMs"], 60_000);
        assert_ne!(host_config["telegram"]["enabled"], true, "a copy never turns Telegram on");
        assert_eq!(std::fs::read_to_string(library.join("Notas/a.md")).expect("host a"), "uno en la copia");
        assert_eq!(std::fs::read_to_string(library.join("c.md")).expect("host c"), "nuevo");
        assert_eq!(std::fs::read_to_string(copy.join("b.md")).expect("copy b"), "dos en el host");

        // Both edit a.md: the last modified wins, whatever side.
        write(&copy.join("Notas/a.md"), "copia vieja", base_ms + 30_000);
        write(&library.join("Notas/a.md"), "host nuevo", base_ms + 40_000);
        // A deletion on the host reaches the copy.
        std::fs::remove_file(library.join("b.md")).expect("delete");
        crate::host::async_runtime::block_on(async { sync(&client).await.expect("conflict") });
        assert_eq!(std::fs::read_to_string(copy.join("Notas/a.md")).expect("copy a"), "host nuevo");
        assert!(!copy.join("b.md").exists());

        write(&copy.join("Notas/a.md"), "copia más nueva", base_ms + 60_000);
        write(&library.join("Notas/a.md"), "host anterior", base_ms + 50_000);
        crate::host::async_runtime::block_on(async { sync(&client).await.expect("conflict") });
        assert_eq!(std::fs::read_to_string(library.join("Notas/a.md")).expect("host a"), "copia más nueva");

        // The database: rows travel both ways. Offline each side adds a
        // note and both change the shared one; the newest change wins.
        let host_db = crate::database::open_library_connection(&library.to_string_lossy()).expect("host database");
        let copy_db = crate::database::open_copy_database_file(&copy.join(".notia/notia.db")).expect("copy database");
        let note = "INSERT INTO agenda_notes (id, owner_user_id, text, created_at, updated_at) VALUES (?1, 'user-owner', ?2, 'x', 'x')";
        let text = |db: &rusqlite::Connection, id: &str| -> Option<String> {
            db.query_row("SELECT text FROM agenda_notes WHERE id = ?1", [id], |row| row.get(0)).ok()
        };
        host_db.execute(note, ["compartida", "original"]).expect("shared note");
        crate::host::async_runtime::block_on(async {
            let status = sync(&client).await.expect("rows down");
            assert!(status.rows_downloaded >= 1, "{status:?}");
        });
        assert_eq!(text(&copy_db, "compartida").as_deref(), Some("original"));
        copy_db.execute(note, ["de-la-copia", "sin conexión"]).expect("copy note");
        host_db.execute(note, ["del-host", "en el host"]).expect("host note");
        copy_db.execute("UPDATE agenda_notes SET text = 'copia más nueva' WHERE id = 'compartida'", []).expect("copy edit");
        host_db.execute("UPDATE agenda_notes SET text = 'host anterior' WHERE id = 'compartida'", []).expect("host edit");
        let at = |db: &rusqlite::Connection, ms: i64| {
            db.execute("UPDATE notia_row_changes SET changed_ms = ?1 WHERE table_name = 'agenda_notes' AND row_key = json_array('compartida')", [ms])
                .expect("time");
        };
        at(&copy_db, base_ms + 2_000);
        at(&host_db, base_ms + 1_000);
        crate::host::async_runtime::block_on(async {
            let status = sync(&client).await.expect("rows both ways");
            assert!(status.rows_uploaded == 2 && status.rows_downloaded >= 1, "{status:?}");
        });
        for db in [&host_db, &copy_db] {
            assert_eq!(text(db, "compartida").as_deref(), Some("copia más nueva"));
            assert_eq!(text(db, "de-la-copia").as_deref(), Some("sin conexión"));
            assert_eq!(text(db, "del-host").as_deref(), Some("en el host"));
        }
        assert!(!crate::row_sync::has_pending(&copy_db).expect("pending"));
        drop((host_db, copy_db));

        // The copy is marked with its library; another library's copy is refused.
        let mark = copy.join(".notia").join("notia-copy.json");
        assert!(std::fs::read_to_string(&mark).expect("mark").contains(&library_id));
        std::fs::write(&mark, r#"{"hostLibraryId":"otra"}"#).expect("other mark");
        let refused = crate::host::async_runtime::block_on(sync(&client));
        assert!(refused.is_err_and(|error| error.message.contains("otra biblioteca")));
        std::fs::write(&mark, format!(r#"{{"hostLibraryId":"{library_id}"}}"#)).expect("mark back");

        // The host gives the library a new id (its catalog made again): the
        // copy goes on under it, with what each side changed meanwhile.
        let new_id = uuid::Uuid::new_v4().to_string();
        let catalog_file = root.join("host").join("library-catalog.json");
        let catalog = std::fs::read_to_string(&catalog_file).expect("catalog").replace(&library_id, &new_id);
        std::fs::write(&catalog_file, catalog).expect("catalog with the new id");
        host.state::<crate::library_registry::LibraryBindingRegistry>()
            .register_desktop_root(&new_id, &library)
            .expect("binding of the new id");
        write(&copy.join("Notas/a.md"), "copia con el id nuevo", base_ms + 70_000);
        write(&library.join("c.md"), "host con el id nuevo", base_ms + 70_000);
        crate::host::async_runtime::block_on(async {
            // The sign-in starts a sync in the background; this one waits for it.
            crate::host_client::auth_login(&client, &new_id, "owner", "contraseña-1", false, false).await.expect("login again");
            sync(&client).await.expect("sync under the new id");
        });
        assert_eq!(std::fs::read_to_string(library.join("Notas/a.md")).expect("host a"), "copia con el id nuevo");
        assert_eq!(std::fs::read_to_string(copy.join("c.md")).expect("copy c"), "host con el id nuevo");
        assert!(std::fs::read_to_string(&mark).expect("mark").contains(&new_id));
        assert!(!record_path(&client, &library_id).expect("old record").exists());
        let record: CopyRecord = read_json(&record_path(&client, &new_id).expect("record"));
        assert!(record.database_id.is_some() && record.folder.as_deref() == Some(copy.as_path()), "{record:?}");
        let current: CurrentCopy = read_json(&directory(&client).expect("directory").join(CURRENT_FILE));
        assert_eq!(current.library_id, new_id);
        assert!(copy_ready(&client));

        stop.store(true, Ordering::SeqCst);
        drop(host);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The window of a client always works on the copy: it opens at once,
    /// with or without the host, changes there, and the sync takes the
    /// changes both ways while it stays open.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn the_window_works_on_the_copy_and_it_syncs_while_open() {
        use crate::backend::connection::{ConnectionSettings, RunMode};
        use crate::host::{AppPaths, HostPorts};

        let root = std::env::temp_dir().join(format!("notia-open-copy-{}", uuid::Uuid::new_v4()));
        let (host, port, stop, library_id) = crate::host_client::test_host::start(&root);
        let library = root.join("Biblioteca");
        std::fs::write(library.join("nota.md"), "hola").expect("note");
        let client = crate::create_app(AppPaths::new(Some(root.join("client")), None), HostPorts::default());
        client.manage(crate::mobile_directory_picker::AndroidDirectoryPickerState::empty());
        let settings = ConnectionSettings { mode: RunMode::Client, host_address: format!("127.0.0.1:{port}"), ..ConnectionSettings::default() };
        crate::connection::store(&client, &settings).expect("settings");
        let open = || {
            let opened = crate::host::async_runtime::block_on(crate::connection::client_open(client.clone())).expect("open");
            serde_json::to_value(&opened).expect("json")
        };

        // Without a copy, the window first signs in with the host.
        assert_eq!(open()["opening"], "first-sync");
        crate::host::async_runtime::block_on(async {
            crate::host_client::auth_login(&client, &library_id, "owner", "contraseña-1", false, false).await.expect("login");
            sync(&client).await.expect("first copy");
        });
        assert!(copy_ready(&client));

        // The copy opens with the host answering, signed in with the
        // session of the first copy.
        assert_eq!(open()["opening"], "copy");
        assert!(is_open(&client));
        let copy_id = crate::library_catalog::selected_library(&client).expect("the copy is selected").id;
        let payload = serde_json::from_value(json!({ "libraryId": copy_id })).expect("payload");
        let status = serde_json::to_value(crate::app_auth::app_auth_status(&client, payload).expect("status")).expect("json");
        assert_eq!(status["state"], "unlocked");

        // The window changes the copy; the next sync takes it to the host,
        // and what the host changed comes down.
        let record: CopyRecord = read_json(&record_path(&client, &library_id).expect("record"));
        let folder = record.folder.expect("copy folder");
        std::fs::write(folder.join("nota.md"), "hola desde la copia").expect("edit");
        let database = crate::database::open_library_connection(&folder.to_string_lossy()).expect("copy database");
        database
            .execute("INSERT INTO agenda_notes (id, owner_user_id, text, created_at, updated_at) VALUES ('n', 'user-owner', 'en la copia', 'x', 'x')", [])
            .expect("write on the copy");
        drop(database);
        std::fs::write(library.join("del-host.md"), "nuevo").expect("host file");
        crate::host::async_runtime::block_on(async {
            let status = sync(&client).await.expect("sync while open");
            assert!(status.uploaded >= 1 && status.downloaded >= 1 && status.rows_uploaded >= 1, "{status:?}");
        });
        assert_eq!(std::fs::read_to_string(library.join("nota.md")).expect("host note"), "hola desde la copia");
        assert_eq!(std::fs::read_to_string(folder.join("del-host.md")).expect("copy file"), "nuevo");
        let host_db = crate::database::open_library_connection(&library.to_string_lossy()).expect("host database");
        let text: String = host_db.query_row("SELECT text FROM agenda_notes WHERE id = 'n'", [], |row| row.get(0)).expect("row");
        assert_eq!(text, "en la copia");
        drop(host_db);

        // The password changes on the host: the copy still has the old one,
        // and signing in with the new one brings it before opening.
        crate::app_auth::host_change_password(&host, "owner", "contraseña-1", "contraseña-2").expect("new password");
        let library_payload = serde_json::from_value(json!({ "libraryId": copy_id })).expect("payload");
        crate::app_auth::app_auth_logout(&client, library_payload).expect("sign out of the copy");
        let login = serde_json::from_value(json!({ "libraryId": copy_id, "username": "owner", "password": "contraseña-2" })).expect("login");
        let signed_in = crate::host::async_runtime::block_on(crate::app_auth::backend_app_auth_login(client.clone(), login)).expect("new password");
        assert_eq!(signed_in["state"], "unlocked");

        // Without the host the copy still opens at once.
        stop.store(true, Ordering::SeqCst);
        drop(host);
        let started = std::time::Instant::now();
        assert_eq!(open()["opening"], "copy");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));

        // Back to Host mode, the copy closes.
        close_copy(&client).expect("close");
        assert!(!is_open(&client));
        let _ = std::fs::remove_dir_all(&root);
    }
}
