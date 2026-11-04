use serde::Deserialize;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[cfg(target_os = "windows")]
const BACKUP_FILE_PREFIX: &str = "notia-backup-";
#[cfg(target_os = "windows")]
const TEMP_FILE_PREFIX: &str = ".notia-backup-";
#[cfg(target_os = "windows")]
const MAX_BACKUP_AGE_SECS: u64 = 2 * 24 * 60 * 60;
#[cfg(target_os = "windows")]
const STALE_TEMP_MAX_AGE_SECS: u64 = 24 * 60 * 60;
#[cfg(target_os = "windows")]
const MAX_BACKUP_COUNT: usize = 48;

#[cfg(target_os = "windows")]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPayload {
    pub library_path: String,
    pub backup_directory: String,
}

#[cfg(target_os = "windows")]
fn create_backup(payload: BackupPayload) -> Result<BackupResult, String> {
    let library = fs::canonicalize(&payload.library_path)
        .map_err(|_| "La biblioteca no es accesible.".to_string())?;
    if !library.is_dir() {
        return Err("La biblioteca no es una carpeta.".into());
    }
    let requested_destination = PathBuf::from(&payload.backup_directory);
    let destination_for_check = if requested_destination.exists() {
        fs::canonicalize(&requested_destination)
            .map_err(|_| "La carpeta de backups no es accesible.".to_string())?
    } else {
        let parent = requested_destination
            .parent()
            .ok_or_else(|| "La carpeta de backups no es válida.".to_string())?;
        fs::canonicalize(parent)
            .map_err(|_| "La carpeta de backups no es accesible.".to_string())?
            .join(
                requested_destination
                    .file_name()
                    .ok_or_else(|| "La carpeta de backups no es válida.".to_string())?,
            )
    };
    if destination_for_check.starts_with(&library) {
        return Err("La carpeta de backups no puede estar dentro de la biblioteca.".into());
    }
    fs::create_dir_all(&requested_destination)
        .map_err(|_| "No se pudo crear la carpeta de backups.".to_string())?;
    let destination = fs::canonicalize(requested_destination)
        .map_err(|_| "La carpeta de backups no es accesible.".to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Reloj del sistema inválido.")?
        .as_secs();
    prune_backups(&destination, timestamp);
    let final_path = destination.join(format!("notia-backup-{timestamp}.zip"));
    let temp_path = destination.join(format!(".notia-backup-{timestamp}.tmp"));
    let result = (|| {
        let file = File::create(&temp_path).map_err(|_| "No se pudo crear el archivo temporal.")?;
        let mut zip = ZipWriter::new(file);
        add_directory(&mut zip, &library, &library)?;
        zip.finish()
            .map_err(|_| "No se pudo cerrar el archivo comprimido.")?;
        fs::rename(&temp_path, &final_path).map_err(|_| "No se pudo guardar el backup.")?;
        Ok::<(), String>(())
    })();
    let _ = fs::remove_file(&temp_path);
    prune_backups(&destination, timestamp);
    result?;
    Ok(BackupResult {
        ok: true,
        error: None,
    })
}

#[cfg(target_os = "windows")]
fn add_directory(zip: &mut ZipWriter<File>, root: &Path, current: &Path) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|_| "No se pudo leer la biblioteca.")? {
        let entry = entry.map_err(|_| "No se pudo leer la biblioteca.")?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Ruta de biblioteca inválida.")?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if entry
            .file_type()
            .map_err(|_| "No se pudo inspeccionar la biblioteca.")?
            .is_dir()
        {
            zip.add_directory(format!("{name}/"), SimpleFileOptions::default())
                .map_err(|_| "No se pudo comprimir la biblioteca.")?;
            add_directory(zip, root, &path)?;
        } else if entry
            .file_type()
            .map_err(|_| "No se pudo inspeccionar la biblioteca.")?
            .is_file()
        {
            let mut input =
                File::open(&path).map_err(|_| "No se pudo leer un archivo de la biblioteca.")?;
            zip.start_file(name, SimpleFileOptions::default())
                .map_err(|_| "No se pudo comprimir la biblioteca.")?;
            io::copy(&mut input, zip)
                .map_err(|_| "No se pudo comprimir un archivo de la biblioteca.")?;
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn prune_backups(destination: &Path, now: u64) {
    let mut backups = Vec::new();
    if let Ok(entries) = fs::read_dir(destination) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if is_backup_temp(file_name) {
                let modified_timestamp = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|age| age.as_secs());
                if is_stale_backup_temp(modified_timestamp, now) {
                    let _ = fs::remove_file(path);
                }
                continue;
            }
            if path.extension().and_then(|v| v.to_str()) != Some("zip")
                || !file_name.starts_with(BACKUP_FILE_PREFIX)
            {
                continue;
            }
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                if let Ok(age) = modified.duration_since(UNIX_EPOCH) {
                    backups.push((age.as_secs(), path));
                }
            }
        }
    }
    backups.sort_by_key(|item| std::cmp::Reverse(item.0));
    let mut retained = 0usize;
    for (created, path) in backups {
        if now.saturating_sub(created) > MAX_BACKUP_AGE_SECS || retained >= MAX_BACKUP_COUNT {
            let _ = fs::remove_file(path);
        } else {
            retained += 1;
        }
    }
}

#[cfg(target_os = "windows")]
fn is_backup_temp(file_name: &str) -> bool {
    file_name.starts_with(TEMP_FILE_PREFIX) && file_name.ends_with(".tmp")
}

#[cfg(target_os = "windows")]
fn is_stale_backup_temp(modified_timestamp: Option<u64>, now: u64) -> bool {
    modified_timestamp
        .map(|modified| now.saturating_sub(modified) > STALE_TEMP_MAX_AGE_SECS)
        .unwrap_or(false)
}

/// Backup service: the destination is chosen with the native picker and
/// stored by the backend; a background scheduler zips the selected library
/// every hour whether or not the WebView is visible.
pub(crate) mod service {
    use std::collections::HashMap;
    use std::sync::Mutex;
    #[cfg(target_os = "windows")]
    use std::time::{Duration, Instant};
    #[cfg(not(target_os = "windows"))]
    use std::time::Instant;

    use serde::{Deserialize, Serialize};
    use crate::host::{AppHandle, Manager};

    use crate::backend::{BackendError, BackendErrorCode};

    const SETTINGS_FILE: &str = "backup-settings.json";
    #[cfg(target_os = "windows")]
    const BACKUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
    #[cfg(target_os = "windows")]
    const SCHEDULER_TICK: Duration = Duration::from_secs(60);

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BackupSettings {
        directory_path: Option<String>,
        /// The legacy client setting was migrated (or the user chose).
        #[serde(default)]
        initialized: bool,
    }

    #[derive(Default)]
    struct BackupRuntime {
        last_attempt: HashMap<String, Instant>,
        last_success_at: Option<u64>,
        last_error: Option<String>,
        running: bool,
    }

    #[derive(Default)]
    pub(crate) struct BackupState {
        runtime: Mutex<BackupRuntime>,
        settings_lock: Mutex<()>,
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct BackupStatusDto {
        supported: bool,
        directory_path: Option<String>,
        initialized: bool,
        last_backup_at: Option<u64>,
        last_error: Option<String>,
    }

    fn storage() -> BackendError {
        BackendError::new(BackendErrorCode::Storage, "No se pudo guardar la configuración de backups.", true)
    }

    fn settings_file(app: &AppHandle) -> Result<std::path::PathBuf, BackendError> {
        app.path().app_data_dir().map(|directory| directory.join(SETTINGS_FILE)).map_err(|_| storage())
    }

    fn read_settings(app: &AppHandle) -> BackupSettings {
        settings_file(app)
            .ok()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write_settings(app: &AppHandle, settings: &BackupSettings) -> Result<(), BackendError> {
        let file = settings_file(app)?;
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|_| storage())?;
        }
        let temporary = file.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(settings).map_err(|_| storage())?)
            .map_err(|_| storage())?;
        std::fs::rename(&temporary, &file).map_err(|_| storage())
    }

    /// A destination inside a registered library would back itself up.
    fn validate_destination(app: &AppHandle, directory: &str) -> Result<(), BackendError> {
        let path = std::path::Path::new(directory);
        if !path.is_absolute() {
            return Err(BackendError::invalid_input("La carpeta de backups no es válida."));
        }
        #[cfg(not(target_os = "android"))]
        if app
            .state::<crate::library_registry::LibraryBindingRegistry>()
            .contains_desktop_path(path)
        {
            return Err(BackendError::invalid_input(
                "La carpeta de backups no puede estar dentro de una biblioteca.",
            ));
        }
        let _ = app;
        Ok(())
    }

    fn status(app: &AppHandle) -> BackupStatusDto {
        let settings = read_settings(app);
        let state = app.state::<BackupState>();
        let (last_backup_at, last_error) = state
            .runtime
            .lock()
            .map(|runtime| (runtime.last_success_at, runtime.last_error.clone()))
            .unwrap_or((None, None));
        BackupStatusDto {
            supported: cfg!(target_os = "windows"),
            directory_path: settings.directory_path,
            initialized: settings.initialized,
            last_backup_at,
            last_error,
        }
    }

    fn update_settings(app: &AppHandle, directory_path: Option<String>) -> Result<BackupStatusDto, BackendError> {
        if let Some(directory) = directory_path.as_deref() {
            validate_destination(app, directory)?;
        }
        let state = app.state::<BackupState>();
        let _guard = state.settings_lock.lock().map_err(|_| storage())?;
        write_settings(app, &BackupSettings { directory_path, initialized: true })?;
        Ok(status(app))
    }

    pub(crate) fn backend_backup_status(app: AppHandle) -> BackupStatusDto {
        status(&app)
    }

    /// Opens the native folder picker; cancelling keeps the settings.
    pub(crate) async fn backend_pick_backup_directory(app: AppHandle) -> Result<BackupStatusDto, BackendError> {
        crate::host::async_runtime::spawn_blocking(move || {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                use crate::host::dialog::DialogExt;
                let Some(selected) = app
                    .dialog()
                    .file()
                    .set_title("Elegí la carpeta para guardar los backups")
                    .blocking_pick_folder()
                else {
                    return Ok(status(&app));
                };
                let path = selected
                    .into_path()
                    .map_err(|_| BackendError::invalid_input("La carpeta de backups no es válida."))?;
                update_settings(&app, Some(path.to_string_lossy().into_owned()))
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                let _ = app;
                Err(BackendError::new(
                    BackendErrorCode::Unsupported,
                    "Los backups solo están disponibles en Windows.",
                    false,
                ))
            }
        })
        .await
        .map_err(|_| storage())?
    }

    pub(crate) fn backend_disable_backups(app: AppHandle) -> Result<BackupStatusDto, BackendError> {
        update_settings(&app, None)
    }

    /// One-time import of the destination chosen in older versions.
    pub(crate) fn backend_migrate_backup_directory(
        app: AppHandle,
        directory_path: Option<String>,
    ) -> Result<BackupStatusDto, BackendError> {
        if read_settings(&app).initialized {
            return Ok(status(&app));
        }
        update_settings(&app, directory_path.filter(|path| !path.trim().is_empty()))
    }

    #[cfg(target_os = "windows")]
    fn run_due_backup(app: &AppHandle) {
        let Some(directory) = read_settings(app).directory_path else {
            return;
        };
        let Some(library_id) = crate::library_catalog::selected_library_id(app) else {
            return;
        };
        let Ok(binding) = app
            .state::<crate::library_registry::LibraryBindingRegistry>()
            .lookup(&library_id)
        else {
            return;
        };
        let Some(crate::library_registry::LibraryBindingRoot::Desktop { canonical_root }) = binding.root else {
            return;
        };
        let key = format!("{library_id}\u{0}{directory}");
        let state = app.state::<BackupState>();
        {
            let Ok(mut runtime) = state.runtime.lock() else {
                return;
            };
            if runtime.running
                || runtime
                    .last_attempt
                    .get(&key)
                    .is_some_and(|attempt| attempt.elapsed() < BACKUP_INTERVAL)
            {
                return;
            }
            runtime.running = true;
            runtime.last_attempt.insert(key, Instant::now());
        }
        let result = super::create_backup(super::BackupPayload {
            library_path: canonical_root.to_string_lossy().into_owned(),
            backup_directory: directory,
        });
        let lock = state.runtime.lock();
        if let Ok(mut runtime) = lock {
            runtime.running = false;
            match result {
                Ok(_) => {
                    runtime.last_error = None;
                    runtime.last_success_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|elapsed| elapsed.as_secs());
                }
                Err(error) => {
                    log::warn!("[notia:backup] el backup automático falló");
                    runtime.last_error = Some(error);
                }
            }
        }
    }

    /// Starts the hourly scheduler (Windows only).
    pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
        crate::host::plugin::Builder::new("library-backups")
            .setup(|app, _api| {
                #[cfg(target_os = "windows")]
                {
                    let app = app.clone();
                    std::thread::spawn(move || loop {
                        std::thread::sleep(SCHEDULER_TICK);
                        // Only the computer that runs the library's services backs it up.
                        if crate::device_role::runs_services(&app) {
                            run_due_backup(&app);
                        }
                    });
                }
                #[cfg(not(target_os = "windows"))]
                let _ = app;
                Ok(())
            })
            .build()
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{
        is_backup_temp, is_stale_backup_temp, MAX_BACKUP_AGE_SECS, STALE_TEMP_MAX_AGE_SECS,
    };

    #[test]
    fn recognizes_only_notia_backup_temporary_files() {
        assert!(is_backup_temp(".notia-backup-123.tmp"));
        assert!(!is_backup_temp("notia-backup-123.zip"));
        assert!(!is_backup_temp(".other-backup-123.tmp"));
    }

    #[test]
    fn temporary_files_expire_before_regular_backups() {
        assert!(STALE_TEMP_MAX_AGE_SECS < MAX_BACKUP_AGE_SECS);
        assert!(is_stale_backup_temp(Some(0), STALE_TEMP_MAX_AGE_SECS + 1));
        assert!(!is_stale_backup_temp(Some(1), STALE_TEMP_MAX_AGE_SECS + 1));
        assert!(!is_stale_backup_temp(None, STALE_TEMP_MAX_AGE_SECS + 1));
    }
}
