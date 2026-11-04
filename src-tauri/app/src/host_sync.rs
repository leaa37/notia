//! What a host gives the copy of a «Con copia» client (`host_mirror`):
//! the list of the library's files with their size and modification time,
//! a consistent snapshot of the database, and writes and deletions of
//! single files. Files are downloaded through `/api/file`.
//!
//! Only the files `mirror_sync::is_synced_path` accepts travel, and every
//! path is checked to stay inside the library.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::backend::mirror_sync::{is_synced_path, FileStamp};
use crate::backend::{BackendError, BackendErrorCode, LogicalPathDto};
use crate::host::{AppHandle, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};

/// Largest file a client uploads (it travels in base64 inside a command).
pub(crate) const MAX_SYNC_UPLOAD_BYTES: u64 = 24 * 1024 * 1024;
const MAX_SYNC_FILES: usize = 100_000;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncFile {
    pub(crate) path: String,
    #[serde(flatten)]
    pub(crate) stamp: FileStamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncManifest {
    /// Folder of the library on the host, to download its files.
    pub(crate) root: String,
    pub(crate) files: Vec<SyncFile>,
    /// The database, when the library has one.
    pub(crate) database: Option<FileStamp>,
    /// Id of the host's database (`row_sync`): the identity of the library
    /// whatever id the catalog gives it.
    #[serde(default)]
    pub(crate) database_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncLibraryPayload {
    library_id: String,
    /// The computer that syncs, for deciding which one runs the library's
    /// services (`device_role`). Android sends none.
    #[serde(default)]
    device: Option<crate::backend::connection::DevicePower>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncWritePayload {
    library_id: String,
    path: String,
    content_base64: String,
    modified_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncDeletePayload {
    library_id: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabaseSnapshot {
    pub(crate) content_base64: String,
    pub(crate) modified_ms: i64,
    /// The id of the host's database and the last change the snapshot
    /// holds: the copy asks for the changes after it.
    #[serde(default)]
    pub(crate) database_id: Option<String>,
    #[serde(default)]
    pub(crate) seq: i64,
}

/// Most row changes in one request, either way.
pub(crate) const MAX_ROW_CHANGES: usize = 500;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabaseChangesPayload {
    library_id: String,
    since: i64,
    /// The host database the copy follows; another one needs a snapshot.
    database_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabaseChanges {
    #[serde(flatten)]
    pub(crate) batch: crate::row_sync::ChangeBatch,
    /// The copy follows another database, or a newer one than this host's:
    /// it needs a snapshot.
    #[serde(default)]
    pub(crate) reset: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabasePushPayload {
    library_id: String,
    changes: Vec<crate::row_sync::RowChange>,
}

fn storage(message: &str) -> BackendError {
    BackendError::new(BackendErrorCode::Storage, message, true)
}

fn library_root(app: &AppHandle, library_id: &str) -> Result<PathBuf, BackendError> {
    match app.state::<LibraryBindingRegistry>().lookup(library_id)?.root {
        Some(LibraryBindingRoot::Desktop { canonical_root }) => Ok(canonical_root),
        _ => Err(BackendError::new(
            BackendErrorCode::Unsupported,
            "Solo un host de Windows o Linux comparte su biblioteca.",
            false,
        )),
    }
}

/// Size and modification time of a file.
pub(crate) fn stamp_of(metadata: &std::fs::Metadata) -> FileStamp {
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default();
    FileStamp { size: metadata.len(), modified_ms }
}

/// Files of `root` the copy keeps, by logical path. Hidden folders other
/// than `.notia` and `.agent` are not entered; links are not followed.
pub(crate) fn synced_files(root: &Path) -> Result<Vec<SyncFile>, BackendError> {
    let mut files = Vec::new();
    walk(root, "", 0, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn walk(directory: &Path, prefix: &str, depth: usize, files: &mut Vec<SyncFile>) -> Result<(), BackendError> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let Ok(children) = std::fs::read_dir(directory) else {
        return Ok(());
    };
    for child in children.filter_map(Result::ok) {
        let name = child.file_name().to_string_lossy().into_owned();
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let Ok(metadata) = std::fs::symlink_metadata(child.path()) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let enters = !name.starts_with('.') || (prefix.is_empty() && (name == ".notia" || name == ".agent"));
            if enters {
                walk(&child.path(), &path, depth + 1, files)?;
            }
            continue;
        }
        if !is_synced_path(&path) || LogicalPathDto::new(&path).is_err() {
            continue;
        }
        if files.len() >= MAX_SYNC_FILES {
            return Err(BackendError::new(BackendErrorCode::Unsupported, "La biblioteca tiene demasiados archivos para copiarla.", false));
        }
        files.push(SyncFile { path, stamp: stamp_of(&metadata) });
    }
    Ok(())
}

/// A logical path that travels between a copy and its host.
pub(crate) fn check_synced_path(path: &str) -> Result<LogicalPathDto, BackendError> {
    let logical = LogicalPathDto::new(path)?;
    if !is_synced_path(logical.as_str()) {
        return Err(BackendError::invalid_input("Ese archivo no se sincroniza."));
    }
    Ok(logical)
}

/// `root` joined with a synced logical path, refused when it leaves `root`.
pub(crate) fn synced_target(root: &Path, path: &str) -> Result<PathBuf, BackendError> {
    let logical = check_synced_path(path)?;
    let target = logical.as_str().split('/').fold(root.to_path_buf(), |path, segment| path.join(segment));
    // An existing folder on the way must not be a link out of the library.
    let mut existing = target.parent().map(Path::to_path_buf);
    while let Some(folder) = existing {
        if folder.exists() {
            let canonical = std::fs::canonicalize(&folder).map_err(|_| storage("No se pudo resolver la carpeta."))?;
            if !canonical.starts_with(root) {
                return Err(BackendError::invalid_input("El archivo queda fuera de la biblioteca."));
            }
            break;
        }
        existing = folder.parent().map(Path::to_path_buf);
    }
    Ok(target)
}

/// Writes `bytes` at `target` with the given modification time.
pub(crate) fn write_synced(target: &Path, bytes: &[u8], modified_ms: i64) -> Result<FileStamp, BackendError> {
    let parent = target.parent().ok_or_else(|| storage("No se pudo resolver la carpeta."))?;
    std::fs::create_dir_all(parent).map_err(|_| storage("No se pudo crear la carpeta."))?;
    let temporary = parent.join(format!(".notia-sync-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(|_| storage("No se pudo guardar el archivo."))?;
    if let Ok(time) = u64::try_from(modified_ms).map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms)) {
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&temporary) {
            let _ = file.set_modified(time);
        }
    }
    std::fs::rename(&temporary, target).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        storage("No se pudo guardar el archivo.")
    })?;
    std::fs::metadata(target).map(|metadata| stamp_of(&metadata)).map_err(|_| storage("No se pudo leer el archivo."))
}

/// Deletes `target` and the folders it leaves empty, up to `root`.
pub(crate) fn delete_synced(root: &Path, target: &Path) -> Result<(), BackendError> {
    match std::fs::remove_file(target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(storage("No se pudo borrar el archivo.")),
    }
    let mut folder = target.parent().map(Path::to_path_buf);
    while let Some(current) = folder {
        if current == root || !current.starts_with(root) || std::fs::remove_dir(&current).is_err() {
            break;
        }
        folder = current.parent().map(Path::to_path_buf);
    }
    Ok(())
}

// ---------- Commands of the host ----------

pub(crate) fn host_sync_manifest(app: &AppHandle, payload: SyncLibraryPayload) -> Result<SyncManifest, BackendError> {
    let root = library_root(app, &payload.library_id)?;
    if let Some(device) = payload.device {
        crate::device_role::note_client(app, device);
    }
    let database_file = root.join(".notia").join("notia.db");
    let database = std::fs::metadata(&database_file).ok().map(|metadata| stamp_of(&metadata));
    let database_id = database.is_some().then(|| stored_database_id(&database_file)).flatten();
    Ok(SyncManifest { root: plain_root(&root), files: synced_files(&root)?, database, database_id })
}

/// The id the database already carries, read as it is (no migration): a
/// database from before the row log has none.
fn stored_database_id(database_file: &Path) -> Option<String> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = rusqlite::Connection::open_with_flags(database_file, flags).ok()?;
    crate::row_sync::database_id(&connection).ok().flatten()
}

/// The root as clients join paths to it: a Windows canonical path loses
/// its verbatim prefix (`\\?\`), which does not accept `/` as separator.
fn plain_root(root: &Path) -> String {
    let text = root.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text.into_owned(),
    }
}

/// The host's database, migrated (with its log of row changes).
#[cfg(any(target_os = "android", target_os = "ios"))]
fn host_database(_root: &Path) -> Result<rusqlite::Connection, BackendError> {
    Err(BackendError::new(BackendErrorCode::Unsupported, "Solo un host de Windows o Linux comparte su biblioteca.", false))
}

/// The host's database, migrated (with its log of row changes).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn host_database(root: &Path) -> Result<rusqlite::Connection, BackendError> {
    let connection = crate::database::open_library_connection(&root.to_string_lossy()).map_err(|message| storage(&message))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| storage("No se pudo abrir la base de datos."))?;
    Ok(connection)
}

fn database_error(_: rusqlite::Error) -> BackendError {
    storage("No se pudo leer o escribir la base de datos de la biblioteca.")
}

/// A consistent copy of the database (`VACUUM INTO`), even while the host
/// writes it. Its log has nothing pending: none of it changed on the copy.
pub(crate) fn host_sync_database(app: &AppHandle, payload: SyncLibraryPayload) -> Result<Option<DatabaseSnapshot>, BackendError> {
    let root = library_root(app, &payload.library_id)?;
    let database = root.join(".notia").join("notia.db");
    let Ok(metadata) = std::fs::metadata(&database) else {
        return Ok(None);
    };
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| storage("No se encontró la carpeta de datos."))?
        .join("host-server");
    std::fs::create_dir_all(&directory).map_err(|_| storage("No se pudo preparar la copia de la base de datos."))?;
    let snapshot = directory.join(format!("snapshot-{}.db", uuid::Uuid::new_v4()));
    let copied = host_database(&root).and_then(|connection| {
        connection.execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()]).map_err(database_error)?;
        drop(connection);
        let copy = rusqlite::Connection::open(&snapshot).map_err(database_error)?;
        crate::row_sync::clear_pending(&copy).map_err(database_error)?;
        let database_id = crate::row_sync::database_id(&copy).map_err(database_error)?;
        let seq = crate::row_sync::last_seq(&copy).map_err(database_error)?;
        drop(copy);
        let bytes = std::fs::read(&snapshot).map_err(|_| storage("No se pudo copiar la base de datos."))?;
        Ok((bytes, database_id, seq))
    });
    let _ = std::fs::remove_file(&snapshot);
    let (bytes, database_id, seq) = copied.map_err(|error| {
        log::error!("[notia:host] the database snapshot failed: {}", error.message);
        storage("No se pudo copiar la base de datos.")
    })?;
    Ok(Some(DatabaseSnapshot {
        content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        modified_ms: stamp_of(&metadata).modified_ms,
        database_id,
        seq,
    }))
}

/// The rows that changed on the host after `since` (`row_sync`).
pub(crate) fn host_sync_database_changes(app: &AppHandle, payload: DatabaseChangesPayload) -> Result<DatabaseChanges, BackendError> {
    let root = library_root(app, &payload.library_id)?;
    let connection = host_database(&root)?;
    let database_id = crate::row_sync::database_id(&connection).map_err(database_error)?;
    let last = crate::row_sync::last_seq(&connection).map_err(database_error)?;
    if payload.database_id.is_none() || payload.database_id != database_id || payload.since > last {
        return Ok(DatabaseChanges { reset: true, ..DatabaseChanges::default() });
    }
    let batch = crate::row_sync::changes_since(&connection, payload.since, MAX_ROW_CHANGES).map_err(database_error)?;
    Ok(DatabaseChanges { batch, reset: false })
}

/// The rows a copy changed while it had no connection: the newest change
/// of each row wins (`row_sync`).
pub(crate) fn host_sync_database_push(app: &AppHandle, payload: DatabasePushPayload) -> Result<crate::row_sync::ApplyReport, BackendError> {
    if payload.changes.len() > MAX_ROW_CHANGES {
        return Err(BackendError::invalid_input("Demasiados cambios de la base en un solo pedido."));
    }
    let root = library_root(app, &payload.library_id)?;
    let connection = host_database(&root)?;
    crate::row_sync::apply_changes(&connection, &payload.changes).map_err(database_error)
}

pub(crate) fn host_sync_write(app: &AppHandle, payload: SyncWritePayload) -> Result<SyncFile, BackendError> {
    let root = library_root(app, &payload.library_id)?;
    let target = synced_target(&root, &payload.path)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.content_base64.as_bytes())
        .map_err(|_| BackendError::invalid_input("El contenido del archivo no es válido."))?;
    if bytes.len() as u64 > MAX_SYNC_UPLOAD_BYTES {
        return Err(BackendError::invalid_input("El archivo supera el tamaño que se sincroniza."));
    }
    // The configuration keeps what only the host decides (Telegram).
    if payload.path == crate::backend::library_config::LIBRARY_CONFIG_LOGICAL_PATH {
        let text = String::from_utf8(bytes).map_err(|_| BackendError::invalid_input("La configuración no es texto válido."))?;
        crate::library_config::merge_client_config(app, &payload.library_id, &text)?;
        let metadata = std::fs::metadata(&target).map_err(|_| storage("No se pudo leer la configuración."))?;
        return Ok(SyncFile { path: payload.path, stamp: stamp_of(&metadata) });
    }
    let stamp = write_synced(&target, &bytes, payload.modified_ms)?;
    Ok(SyncFile { path: payload.path, stamp })
}

pub(crate) fn host_sync_delete(app: &AppHandle, payload: SyncDeletePayload) -> Result<(), BackendError> {
    let root = library_root(app, &payload.library_id)?;
    let target = synced_target(&root, &payload.path)?;
    delete_synced(&root, &target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_writes_and_deletes_only_inside_the_library() {
        let base = std::env::temp_dir().join(format!("notia-host-sync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("Biblioteca/Notas")).expect("folders");
        std::fs::create_dir_all(base.join("Biblioteca/.git")).expect("git");
        std::fs::create_dir_all(base.join("Biblioteca/.notia")).expect("notia");
        let root = std::fs::canonicalize(base.join("Biblioteca")).expect("root");
        std::fs::write(root.join("Notas/idea.md"), "hola").expect("note");
        std::fs::write(root.join(".git/config"), "x").expect("git file");
        std::fs::write(root.join(".notia/notia.db"), "db").expect("db");
        std::fs::write(root.join(".notia/notia.db.pre-v26.sqlite"), "old db").expect("db backup");
        std::fs::write(root.join(".notia/notiaConfig.json"), "{}").expect("config");
        std::fs::create_dir_all(root.join(".notia/ink/Notas")).expect("ink");
        std::fs::write(root.join(".notia/ink/Notas/idea.md.json"), "{}").expect("ink file");

        let files: Vec<String> = synced_files(&root).expect("files").into_iter().map(|file| file.path).collect();
        assert_eq!(files, vec![".notia/ink/Notas/idea.md.json", ".notia/notiaConfig.json", "Notas/idea.md"]);

        let target = synced_target(&root, "Nueva/carpeta/a.md").expect("target");
        let stamp = write_synced(&target, b"texto", 1_700_000_000_000).expect("write");
        assert_eq!(stamp, FileStamp { size: 5, modified_ms: 1_700_000_000_000 });
        delete_synced(&root, &target).expect("delete");
        assert!(!root.join("Nueva").exists(), "empty folders are removed");

        for refused in ["../fuera.md", ".notia/notia.db", ".git/config", "/abs.md", "a\\b.md"] {
            assert!(synced_target(&root, refused).is_err(), "{refused}");
        }
        assert_eq!(plain_root(Path::new(r"\\?\C:\Notas")), r"C:\Notas");
        assert_eq!(plain_root(Path::new("/home/notas")), "/home/notas");
        let _ = std::fs::remove_dir_all(&base);
    }
}
