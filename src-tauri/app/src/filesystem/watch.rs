use serde::{Deserialize, Serialize};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::host::{Emitter, Manager};
use crate::host::{State, Window};

use super::types::OperationResult;

/// The files of the library changed: the explorer reads them again.
pub(crate) const LIBRARY_TREE_CHANGED_EVENT: &str = "notia-library-tree-changed";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartLibraryTreeWatchPayload {
    pub(crate) directory_path: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LibraryTreeChangedEventPayload {
    watched_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_path_hint: Option<String>,
}

#[derive(Default)]
pub struct LibraryTreeWatchState {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    watchers_by_window_label:
        std::sync::Mutex<std::collections::HashMap<String, ActiveLibraryTreeWatch>>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
struct ActiveLibraryTreeWatch {
    watched_path: String,
    _watcher: notify::RecommendedWatcher,
}

fn ok_result() -> OperationResult {
    OperationResult {
        ok: true,
        error: None,
    }
}

fn error_result(message: &str) -> OperationResult {
    OperationResult {
        ok: false,
        error: Some(message.to_string()),
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn should_emit_tree_change_event(event: &notify::Event) -> bool {
    matches!(
        &event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
            | notify::EventKind::Modify(notify::event::ModifyKind::Data(_))
            | notify::EventKind::Modify(notify::event::ModifyKind::Metadata(_))
            | notify::EventKind::Modify(notify::event::ModifyKind::Any)
    )
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn is_internal_notia_path(watched_path: &str, changed_path: &std::path::Path) -> bool {
    let watched = std::path::Path::new(watched_path);
    let Ok(relative) = changed_path.strip_prefix(watched) else {
        return false;
    };
    relative
        .components()
        .next()
        .is_some_and(|component| component.as_os_str() == std::ffi::OsStr::new(".notia"))
}

pub fn start_library_tree_watch(
    window: Window,
    payload: StartLibraryTreeWatchPayload,
    watch_state: State<'_, LibraryTreeWatchState>,
) -> OperationResult {
    if payload.directory_path.trim().is_empty() {
        return error_result("Invalid directory path.");
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use notify::{Config, RecursiveMode, Watcher};
        use std::path::Path;

        let watched_path = payload.directory_path.trim().to_string();
        let window_label = window.label().to_string();
        let app_handle = window.app_handle().clone();
        let watched_path_for_events = watched_path.clone();

        let mut watcher = match notify::RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else {
                    return;
                };

                if !should_emit_tree_change_event(&event) {
                    return;
                }

                let Some(changed_path_hint) = event.paths.iter().find_map(|path| {
                    if is_internal_notia_path(&watched_path_for_events, path) {
                        return None;
                    }
                    let path_value = path.to_string_lossy().trim().to_string();
                    if path_value.is_empty() {
                        None
                    } else {
                        Some(path_value)
                    }
                }) else {
                    return;
                };

                let _ = app_handle.emit(
                    LIBRARY_TREE_CHANGED_EVENT,
                    LibraryTreeChangedEventPayload {
                        watched_path: watched_path_for_events.clone(),
                        changed_path_hint: Some(changed_path_hint),
                    },
                );
            },
            Config::default(),
        ) {
            Ok(watcher) => watcher,
            Err(_) => return error_result("Could not start library tree watch."),
        };

        if watcher
            .watch(Path::new(&watched_path), RecursiveMode::Recursive)
            .is_err()
        {
            return error_result("Could not start library tree watch.");
        }

        let mut watchers_by_window_label = match watch_state.watchers_by_window_label.lock() {
            Ok(guard) => guard,
            Err(_) => return error_result("Could not start library tree watch."),
        };

        watchers_by_window_label.insert(
            window_label,
            ActiveLibraryTreeWatch {
                watched_path,
                _watcher: watcher,
            },
        );

        return ok_result();
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = window;
        let _ = watch_state;
        error_result("Filesystem watch is not available on this platform.")
    }
}

pub fn stop_library_tree_watch(
    window: Window,
    watch_state: State<'_, LibraryTreeWatchState>,
) -> OperationResult {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let mut watchers_by_window_label = match watch_state.watchers_by_window_label.lock() {
            Ok(guard) => guard,
            Err(_) => return error_result("Could not stop library tree watch."),
        };

        if let Some(active_watch) = watchers_by_window_label.remove(window.label()) {
            let _ = active_watch.watched_path;
        }

        return ok_result();
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = window;
        let _ = watch_state;
        ok_result()
    }
}
