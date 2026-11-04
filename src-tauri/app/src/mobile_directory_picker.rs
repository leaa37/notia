use crate::library_registry::LibraryBindingRegistry;
use crate::notia_timer::NotiaTimer;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
#[cfg(target_os = "android")]
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "android")]
use std::sync::Mutex;
#[cfg(target_os = "android")]
use std::time::Instant;
#[cfg(target_os = "android")]
use crate::host::plugin::PluginHandle;
use crate::host::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Manager, State, Wry,
};

/// TTL in seconds for the SAF tree path cache. When a cache entry is younger
/// than this, `refresh_android_tree_path_cache` will skip the expensive
/// recursive `readTree` call and reuse the cached path→URI mappings.
/// Increased from 5s to 30s to reduce redundant full-tree traversals on
/// Android SAF, especially during library add/switch flows.
#[cfg(target_os = "android")]
const SAF_CACHE_TTL_SECS: u64 = 30;

/// Maximum entries kept in the in-memory LRU cache for SAF path→URI lookups.
/// This is a lightweight Rust-side cache that avoids repeated JNI calls and
/// guards against short-term bursts of `resolve_entry_uri` requests.
#[cfg(target_os = "android")]
const SAF_PATH_LRU_CAPACITY: usize = 500;
/// Upper bound of cached logical path → document URI entries across all
/// trees. Beyond it, paths are resolved lazily through SAF instead of cached.
#[cfg(target_os = "android")]
const MAX_SAF_PATH_CACHE_ENTRIES: usize = 20_000;

pub struct AndroidDirectoryPickerState {
    #[cfg(target_os = "android")]
    handle: Mutex<Option<PluginHandle<Wry>>>,
    #[cfg(target_os = "android")]
    roots: Mutex<HashMap<String, String>>,
    #[cfg(target_os = "android")]
    paths: Mutex<HashMap<String, String>>,
    #[cfg(target_os = "android")]
    root_entries: Mutex<HashMap<String, Vec<String>>>,
    /// Tracks the last successful cache refresh time per tree URI so that
    /// repetitive refreshes within the TTL window can be skipped.
    #[cfg(target_os = "android")]
    last_cache_refresh_at: Mutex<HashMap<String, Instant>>,
    /// Per-tree mutation epoch. A tree read that started before a mutation
    /// carries an older epoch and is discarded instead of re-caching stale
    /// path → URI mappings.
    #[cfg(target_os = "android")]
    cache_epochs: Mutex<HashMap<String, u64>>,
    /// Lightweight LRU cache for resolved path→URI lookups to avoid repeated
    /// JNI calls during bursts (e.g. reading many files from the same subtree).
    #[cfg(target_os = "android")]
    saf_path_lru: Mutex<lru::LruCache<String, String>>,
}

// Manual implementation avoids deriving Clone on the inner LruCache, which is
// not needed for shared state and would add an unnecessary bound.
#[cfg(not(target_os = "android"))]
impl Clone for AndroidDirectoryPickerState {
    fn clone(&self) -> Self {
        Self {}
    }
}

#[cfg(any(target_os = "android", test))]
fn normalize_android_root_key(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

#[cfg(any(target_os = "android", test))]
fn is_same_or_nested_path(base: &str, candidate: &str) -> bool {
    let normalized_base = normalize_android_root_key(base);
    let normalized_candidate = normalize_android_root_key(candidate);
    !normalized_base.is_empty()
        && (normalized_candidate == normalized_base
            || normalized_candidate.starts_with(&format!("{normalized_base}/")))
}

#[cfg(target_os = "android")]
fn is_saf_tree_uri(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("content://")
        && trimmed.contains("/tree/")
        && !trimmed.contains("/document/")
        && !trimmed.chars().any(char::is_whitespace)
}

#[cfg(target_os = "android")]
fn validate_tree_context(uri: Option<&str>) -> Result<(), String> {
    if let Some(value) = uri {
        if !is_saf_tree_uri(value) {
            return Err("La referencia Android no es una URI tree SAF válida.".to_string());
        }
    }
    Ok(())
}

impl AndroidDirectoryPickerState {
    #[cfg(target_os = "android")]
    fn with_handle(handle: PluginHandle<Wry>) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
            roots: Mutex::new(HashMap::new()),
            paths: Mutex::new(HashMap::new()),
            root_entries: Mutex::new(HashMap::new()),
            last_cache_refresh_at: Mutex::new(HashMap::new()),
            cache_epochs: Mutex::new(HashMap::new()),
            saf_path_lru: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(SAF_PATH_LRU_CAPACITY).unwrap(),
            )),
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn unavailable() -> Self {
        Self {
            handle: Mutex::new(None),
            roots: Mutex::new(HashMap::new()),
            paths: Mutex::new(HashMap::new()),
            root_entries: Mutex::new(HashMap::new()),
            last_cache_refresh_at: Mutex::new(HashMap::new()),
            cache_epochs: Mutex::new(HashMap::new()),
            saf_path_lru: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(SAF_PATH_LRU_CAPACITY).unwrap(),
            )),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub(crate) fn empty() -> Self {
        Self {}
    }
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PickDirectoryTreeResponse {
    path: Option<String>,
    uri: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickAndroidDirectoryTreeResult {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickAndroidDirectoryTreePayload {
    #[serde(default)]
    pub library_id: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadTreeResponse {
    nodes: Vec<AndroidTreeNode>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileResponse {
    content: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteFileResponse {
    ok: bool,
    /// The document as the provider has it after the write, when it says.
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    last_modified: Option<i64>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyDocumentResponse {
    ok: bool,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateEntryResponse {
    path: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryMutationResponse {
    ok: Option<bool>,
    path: Option<String>,
}

#[cfg(target_os = "android")]
fn cache_android_path(
    path_map: &mut HashMap<String, String>,
    cached_keys: &mut HashSet<String>,
    path: &str,
    uri: &str,
) {
    if path_map.len() >= MAX_SAF_PATH_CACHE_ENTRIES && !path_map.contains_key(path) {
        return;
    }
    path_map.insert(path.to_string(), uri.to_string());
    cached_keys.insert(path.to_string());

    let normalized_path = normalize_android_root_key(path);
    if !normalized_path.is_empty() {
        path_map.insert(normalized_path.clone(), uri.to_string());
        cached_keys.insert(normalized_path);
    }
}

#[cfg(target_os = "android")]
fn cache_android_tree_nodes(
    path_map: &mut HashMap<String, String>,
    cached_keys: &mut HashSet<String>,
    nodes: &[AndroidTreeNode],
) {
    for node in nodes {
        if let Some(path) = node.path.as_deref() {
            if node.id.starts_with("content://") {
                cache_android_path(path_map, cached_keys, path, &node.id);
            }
        }

        if let Some(children) = node.children.as_deref() {
            cache_android_tree_nodes(path_map, cached_keys, children);
        }
    }
}

#[cfg(target_os = "android")]
pub fn is_cache_fresh(state: &AndroidDirectoryPickerState, tree_uri: &str) -> bool {
    let Ok(timestamps) = state.last_cache_refresh_at.lock() else {
        return false;
    };
    timestamps
        .get(tree_uri)
        .map(|instant| instant.elapsed().as_secs() < SAF_CACHE_TTL_SECS)
        .unwrap_or(false)
}

#[cfg(target_os = "android")]
fn mark_cache_fresh(state: &AndroidDirectoryPickerState, tree_uri: &str) {
    if let Ok(mut timestamps) = state.last_cache_refresh_at.lock() {
        timestamps.insert(tree_uri.to_string(), Instant::now());
    }
}

/// Mark the SAF path cache as stale for the given tree URI without doing any
/// I/O. The next call to `refresh_android_tree_path_cache` with `force=false`
/// will perform a full `readTree` to rebuild the cache.
#[cfg(target_os = "android")]
pub fn invalidate_tree_cache(state: &AndroidDirectoryPickerState, tree_uri: &str) {
    if let Ok(mut timestamps) = state.last_cache_refresh_at.lock() {
        timestamps.remove(tree_uri);
    }
}

/// Current mutation epoch of a tree. Capture it before reading the tree and
/// pass it to the cache update so a concurrent mutation wins.
#[cfg(target_os = "android")]
pub fn tree_cache_epoch(state: &AndroidDirectoryPickerState, tree_uri: &str) -> u64 {
    state
        .cache_epochs
        .lock()
        .map(|epochs| epochs.get(tree_uri).copied().unwrap_or(0))
        .unwrap_or(0)
}

/// Writes and entry changes this process made in library folders. The sync
/// of a client's copy lists its SAF folder again only after one.
#[cfg(target_os = "android")]
static LIBRARY_WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(target_os = "android")]
pub(crate) fn library_writes() -> u64 {
    LIBRARY_WRITES.load(std::sync::atomic::Ordering::SeqCst)
}

/// Marks a tree as mutated: the cache becomes stale and every tree read that
/// started earlier is discarded when it tries to update the cache.
#[cfg(target_os = "android")]
pub fn mark_tree_mutated(state: &AndroidDirectoryPickerState, tree_uri: &str) {
    LIBRARY_WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut epochs) = state.cache_epochs.lock() {
        let epoch = epochs.entry(tree_uri.to_string()).or_insert(0);
        *epoch = epoch.wrapping_add(1);
    }
    invalidate_tree_cache(state, tree_uri);
}

#[cfg(not(target_os = "android"))]
pub fn invalidate_paths_by_prefix(_state: &AndroidDirectoryPickerState, _path_prefix: &str) {
    // No-op on non-Android platforms
}

/// Invalidate only the path entries that start with `path_prefix` from the
/// cached paths map, instead of clearing the entire tree cache. This allows
/// subsequent reads of sibling or parent paths to still hit the cache.
#[cfg(target_os = "android")]
pub fn invalidate_paths_by_prefix(state: &AndroidDirectoryPickerState, path_prefix: &str) {
    let normalized_prefix = normalize_android_root_key(path_prefix);
    let mut paths = match state.paths.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    paths.retain(|key, _| !is_same_or_nested_path(&normalized_prefix, key));
    drop(paths);
    invalidate_android_path_lru(state, path_prefix);
    invalidate_android_path_lru(state, &normalized_prefix);
}

/// Lightweight, Rust-only LRU cache for resolved SAF paths. Does not require
/// JNI so it is suitable for hot lookups during bulk reads.
#[cfg(target_os = "android")]
pub fn resolve_android_path_lru(state: &AndroidDirectoryPickerState, path: &str) -> Option<String> {
    let mut lru = state.saf_path_lru.lock().ok()?;
    lru.get(path).cloned()
}

#[cfg(target_os = "android")]
pub fn put_android_path_lru(state: &AndroidDirectoryPickerState, path: String, uri: String) {
    if let Ok(mut lru) = state.saf_path_lru.lock() {
        lru.put(path, uri);
    }
}

#[cfg(target_os = "android")]
pub fn invalidate_android_path_lru(state: &AndroidDirectoryPickerState, path_prefix: &str) {
    if let Ok(mut lru) = state.saf_path_lru.lock() {
        let keys_to_remove: Vec<String> = lru
            .iter()
            .filter(|(key, _)| is_same_or_nested_path(path_prefix, key))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys_to_remove {
            lru.pop(&key);
        }
    }
}

/// Resolves `segments` under `tree_uri` from the deepest folder already
/// known, with one shallow listing per missing level, instead of reading
/// the whole tree. `Some(None)` means a listing showed the entry does not
/// exist; `None` that the walk could not decide (an error), and the caller
/// may still read the whole tree. Found entries go to the LRU unless the
/// tree changed meanwhile. Only exact cached paths are used: a nested path
/// never falls back to the root.
#[cfg(target_os = "android")]
pub fn resolve_android_path_by_walking(
    state: &AndroidDirectoryPickerState,
    tree_uri: &str,
    segments: &[String],
) -> Option<Option<String>> {
    let root = tree_uri.trim_end_matches('/');
    let key = |depth: usize| {
        if depth == 0 {
            root.to_string()
        } else {
            format!("{root}/{}", segments[..depth].join("/"))
        }
    };
    let known = |path: &str| {
        resolve_android_path_lru(state, path).or_else(|| state.paths.lock().ok()?.get(path).cloned())
    };
    let epoch = tree_cache_epoch(state, tree_uri);
    let mut depth = segments.len();
    let mut current = loop {
        if depth == 0 {
            break tree_uri.to_string();
        }
        if let Some(uri) = known(&key(depth)) {
            if depth == segments.len() {
                return Some(Some(uri));
            }
            break uri;
        }
        depth -= 1;
    };
    while depth < segments.len() {
        let listing = {
            let guard = state.handle.lock().ok()?;
            let handle = guard.as_ref()?;
            handle
                .run_mobile_plugin::<ReadTreeResponse>("readDirectory", serde_json::json!({ "uri": current }))
                .ok()?
        };
        let Some(child) = listing.nodes.into_iter().find(|node| node.name == segments[depth]) else {
            return Some(None);
        };
        if !child.id.starts_with("content://") {
            return None;
        }
        depth += 1;
        if tree_cache_epoch(state, tree_uri) == epoch {
            put_android_path_lru(state, key(depth), child.id.clone());
        }
        current = child.id;
    }
    Some(Some(current))
}

/// The children of one folder document (shallow), as `(name, is_file)`.
#[cfg(target_os = "android")]
pub(crate) fn list_android_folder(state: &AndroidDirectoryPickerState, folder_uri: &str) -> Result<Vec<(String, bool)>, String> {
    let guard = state.handle.lock().map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let handle = guard.as_ref().ok_or_else(|| "El selector de carpetas no esta disponible.".to_string())?;
    let response = handle
        .run_mobile_plugin::<ReadTreeResponse>("readDirectory", serde_json::json!({ "uri": folder_uri }))
        .map_err(|error| format!("No se pudo leer el directorio Android: {error}"))?;
    Ok(response.nodes.into_iter().map(|node| (node.name, node.node_type == "file")).collect())
}

/// Update cached path→URI mappings from tree nodes that have already been
/// fetched (e.g. by `read_android_library_tree`). This avoids the redundant
/// second `readTree` call that `refresh_android_tree_path_cache` would make.
#[cfg(target_os = "android")]
pub fn update_cache_from_nodes(
    state: &AndroidDirectoryPickerState,
    tree_uri: &str,
    nodes: &[AndroidTreeNode],
    epoch_before_read: u64,
) {
    let root_paths = {
        let Ok(roots) = state.roots.lock() else {
            return;
        };
        roots
            .iter()
            .filter(|(_, uri)| uri.as_str() == tree_uri)
            .map(|(path, uri)| (path.clone(), uri.clone()))
            .collect::<Vec<_>>()
    };

    let mut cached_keys = HashSet::new();

    let Ok(mut paths) = state.paths.lock() else {
        return;
    };
    let Ok(mut root_entries) = state.root_entries.lock() else {
        return;
    };
    // Checked while holding the cache locks: a mutation that finished after
    // the read started invalidates this snapshot.
    if tree_cache_epoch(state, tree_uri) != epoch_before_read {
        log::debug!("[notia:saf] lectura de árbol obsoleta descartada");
        return;
    }

    if let Some(previous_keys) = root_entries.get(tree_uri) {
        for key in previous_keys {
            paths.remove(key);
        }
    }

    for (root_path, root_uri) in &root_paths {
        cache_android_path(&mut paths, &mut cached_keys, root_path, root_uri);
    }

    cache_android_tree_nodes(&mut paths, &mut cached_keys, nodes);
    root_entries.insert(tree_uri.to_string(), cached_keys.into_iter().collect());
    mark_cache_fresh(state, tree_uri);
}

#[cfg(target_os = "android")]
pub fn refresh_android_tree_path_cache(
    state: &AndroidDirectoryPickerState,
    tree_uri: &str,
    force: bool,
) -> Result<(), String> {
    if !force && is_cache_fresh(state, tree_uri) {
        return Ok(());
    }
    let epoch_before_read = tree_cache_epoch(state, tree_uri);

    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<ReadTreeResponse>("readTree", serde_json::json!({ "uri": tree_uri }))
        .map_err(|error| format!("No se pudo leer la carpeta Android: {error}"))?;

    // Release the handle lock before acquiring paths/root_entries.
    drop(guard);

    let root_paths = {
        let roots = state
            .roots
            .lock()
            .map_err(|_| "No se pudo acceder a las carpetas Android seleccionadas.".to_string())?;
        roots
            .iter()
            .filter(|(_, uri)| uri.as_str() == tree_uri)
            .map(|(path, uri)| (path.clone(), uri.clone()))
            .collect::<Vec<_>>()
    };

    let mut cached_keys = HashSet::new();
    let mut paths = state
        .paths
        .lock()
        .map_err(|_| "No se pudo actualizar la cache Android.".to_string())?;
    let mut root_entries = state
        .root_entries
        .lock()
        .map_err(|_| "No se pudo actualizar la cache Android.".to_string())?;
    if tree_cache_epoch(state, tree_uri) != epoch_before_read {
        log::debug!("[notia:saf] lectura de árbol obsoleta descartada");
        return Ok(());
    }

    if let Some(previous_keys) = root_entries.get(tree_uri) {
        for key in previous_keys {
            paths.remove(key);
        }
    }

    for (root_path, root_uri) in &root_paths {
        cache_android_path(&mut paths, &mut cached_keys, root_path, root_uri);
    }

    cache_android_tree_nodes(&mut paths, &mut cached_keys, &response.nodes);
    root_entries.insert(tree_uri.to_string(), cached_keys.into_iter().collect());
    mark_cache_fresh(state, tree_uri);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AndroidTreeNode {
    id: String,
    name: String,
    path: Option<String>,
    #[serde(rename = "type")]
    node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expanded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_children: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<AndroidTreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<i64>,
}

#[cfg(target_os = "android")]
fn sort_android_tree_nodes(nodes: &mut [AndroidTreeNode]) {
    nodes.sort_by(|a, b| {
        if a.node_type != b.node_type {
            return if a.node_type == "folder" {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let a_date = a.created_at.or(a.modified_at).unwrap_or(i64::MAX);
        let b_date = b.created_at.or(b.modified_at).unwrap_or(i64::MAX);
        let date_cmp = a_date.cmp(&b_date);
        if date_cmp != Ordering::Equal {
            return date_cmp;
        }
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    });
}

/// A flat file entry returned by the `readFlatFileList` Kotlin command.
/// Unlike `AndroidTreeNode`, this has no nesting — just the path, type,
/// and name for every file/folder in the tree. Used by search index and
/// graph engine which need the complete file list without tree structure.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AndroidFlatFileEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: String,
    /// Bytes of a file, when the provider reports it.
    #[serde(default)]
    pub size: Option<u64>,
    /// Modification time of a file (ms since the epoch), when reported.
    #[serde(default)]
    pub last_modified: Option<i64>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFlatFileListResponse {
    files: Vec<AndroidFlatFileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadAndroidTreePayload {
    pub(crate) directory_path: String,
    #[serde(default)]
    pub(crate) directory_uri: Option<String>,
}

pub fn pick_android_directory_tree(
    state: State<'_, AndroidDirectoryPickerState>,
    registry: State<'_, LibraryBindingRegistry>,
    payload: Option<PickAndroidDirectoryTreePayload>,
) -> Result<PickAndroidDirectoryTreeResult, String> {
    #[cfg(target_os = "android")]
    {
        let _timer = NotiaTimer::new("pick_android_directory_tree");
        log::info!("[notia:directory_picker] pick_android_directory_tree called");
        let guard = state
            .handle
            .lock()
            .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
        let Some(handle) = guard.as_ref() else {
            log::warn!("[notia:directory_picker] plugin handle not available");
            return Err("El selector de carpetas no esta disponible.".to_string());
        };

        log::info!("[notia:directory_picker] calling pickDirectoryTree plugin");
        let response = handle
            .run_mobile_plugin::<PickDirectoryTreeResponse>("pickDirectoryTree", ())
            .map_err(|error| {
                log::error!("[notia:directory_picker] plugin call failed: {}", error);
                format!("No se pudo abrir el selector de carpetas: {error}")
            })?;
        log::info!("[notia:directory_picker] plugin response received");
        let Some(path) = response.path else {
            log::warn!("[notia:directory_picker] no path in response");
            return Err("No se pudo resolver la carpeta seleccionada.".to_string());
        };
        if path.trim().is_empty() {
            log::warn!("[notia:directory_picker] path is empty");
            return Err("No se pudo resolver la carpeta seleccionada.".to_string());
        }

        let selected_uri = response
            .uri
            .clone()
            .filter(|uri| is_saf_tree_uri(uri) && uri.trim() == path.trim())
            .ok_or_else(|| "El selector no devolvió una URI tree SAF válida.".to_string())?;

        if let Some(library_id) = payload.and_then(|value| value.library_id) {
            registry
                .register_android_tree(&library_id, &selected_uri)
                .map_err(|error| error.message)?;
        }
        {
            let uri = selected_uri.clone();
            let mut roots = state
                .roots
                .lock()
                .map_err(|_| "No se pudo guardar la referencia de carpeta Android.".to_string())?;
            let normalized_key = normalize_android_root_key(&path);
            roots.insert(path.clone(), uri.clone());
            roots.insert(normalized_key, uri.clone());

            // Invalidate the Rust-side cache for this tree URI so that the
            // next readTree will fetch fresh data (the directory may have
            // changed externally since the last time it was accessed).
            invalidate_tree_cache(state.inner(), &uri);
        }

        {
            let uri = selected_uri.as_str();
            let mut paths = state
                .paths
                .lock()
                .map_err(|_| "No se pudo guardar la referencia de carpeta Android.".to_string())?;
            let normalized_key = normalize_android_root_key(&path);
            paths.insert(path.clone(), uri.to_string());
            paths.insert(normalized_key, uri.to_string());
        }

        return Ok(PickAndroidDirectoryTreeResult {
            path,
            uri: Some(selected_uri),
        });
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (state, registry, payload);
        Err("El selector de carpetas Android solo esta disponible en Android.".to_string())
    }
}

pub async fn read_android_library_tree(
    state: State<'_, AndroidDirectoryPickerState>,
    payload: ReadAndroidTreePayload,
) -> Result<Vec<AndroidTreeNode>, String> {
    #[cfg(target_os = "android")]
    {
        let mut timer = NotiaTimer::new("read_android_library_tree");
        if payload.directory_path.trim().is_empty() {
            return Ok(Vec::new());
        }
        validate_tree_context(payload.directory_uri.as_deref())?;

        log::info!("[notia:directory_picker] read_android_library_tree started");

        let uri = {
            let normalized_key = normalize_android_root_key(&payload.directory_path);
            if let Some(uri) = payload
                .directory_uri
                .clone()
                .filter(|value| !value.trim().is_empty())
            {
                let mut roots = state.roots.lock().map_err(|_| {
                    "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                })?;
                roots.insert(payload.directory_path.clone(), uri.clone());
                roots.insert(normalized_key, uri.clone());
                log::info!("[notia:directory_picker] uri resolved from payload");
                Some(uri)
            } else {
                let exact_path_uri = {
                    let paths = state.paths.lock().map_err(|_| {
                        "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                    })?;
                    paths.get(&payload.directory_path).cloned().or_else(|| {
                        paths
                            .get(&normalize_android_root_key(&payload.directory_path))
                            .cloned()
                    })
                };
                if exact_path_uri.is_some() {
                    exact_path_uri
                } else {
                    let normalized_key = normalize_android_root_key(&payload.directory_path);
                    let roots = state.roots.lock().map_err(|_| {
                        "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                    })?;
                    roots
                        .get(&payload.directory_path)
                        .cloned()
                        .or_else(|| roots.get(&normalized_key).cloned())
                        .or_else(|| {
                            // Multi-root fallback: find the root whose path is a
                            // prefix of the requested path (longest prefix wins).
                            roots
                                .iter()
                                .filter(|(root_path, _)| {
                                    let norm_root = normalize_android_root_key(root_path);
                                    is_same_or_nested_path(root_path, &payload.directory_path)
                                        || is_same_or_nested_path(
                                            &norm_root,
                                            &payload.directory_path,
                                        )
                                })
                                .max_by_key(|(root_path, _)| root_path.len())
                                .map(|(_, uri)| uri.clone())
                        })
                }
            }
        };

        let Some(uri) = uri else {
            log::warn!("[notia:directory_picker] uri resolution failed");
            return Err(
                "No se encontro la referencia Android de la carpeta seleccionada.".to_string(),
            );
        };

        log::info!("[notia:directory_picker] calling readTree plugin");

        // Fetch the tree and update the cache in one pass — no second readTree.
        // This is an async command so the Tauri runtime can process other events
        // (e.g. WebView rendering) while this command runs on the async thread pool.
        let epoch_before_read = tree_cache_epoch(state.inner(), &uri);
        let response = {
            let guard = state
                .handle
                .lock()
                .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
            let Some(handle) = guard.as_ref() else {
                return Err("El selector de carpetas no esta disponible.".to_string());
            };

            handle
                .run_mobile_plugin::<ReadTreeResponse>(
                    "readTree",
                    serde_json::json!({ "uri": uri }),
                )
                .map_err(|error| format!("No se pudo leer la carpeta Android: {error}"))?
        };

        // Update the path cache from the tree nodes we already fetched, avoiding
        // the redundant second readTree that refresh_android_tree_path_cache would do.
        update_cache_from_nodes(state.inner(), &uri, &response.nodes, epoch_before_read);

        let mut nodes = response.nodes;
        sort_android_tree_nodes(&mut nodes);
        let node_count = nodes.len();
        timer.finish_with_meta(&format!("node_count={node_count}"));

        Ok(nodes)
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        let ReadAndroidTreePayload {
            directory_path,
            directory_uri,
        } = payload;
        let _ = (directory_path, directory_uri);
        Err("Solo disponible en Android.".to_string())
    }
}

/// Read a single directory's immediate children (non-recursive, ls-style).
/// Folders are returned with `hasChildren: true` and `children: None`,
/// meaning the caller must issue a separate read_android_directory call
/// to expand them. This avoids the expensive recursive tree traversal on
/// large libraries.
pub async fn read_android_directory(
    state: State<'_, AndroidDirectoryPickerState>,
    payload: ReadAndroidTreePayload,
) -> Result<Vec<AndroidTreeNode>, String> {
    #[cfg(target_os = "android")]
    {
        let mut timer = NotiaTimer::new("read_android_directory");
        if payload.directory_path.trim().is_empty() {
            return Ok(Vec::new());
        }
        validate_tree_context(payload.directory_uri.as_deref())?;

        log::info!("[notia:directory_picker] read_android_directory started");

        // Resolve the URI for this directory.
        // Priority: cached paths > directory_uri payload > roots fallback.
        // This is critical for subdirectory reads: when expanding a subfolder
        // in the tree, directoryUri may be the library root tree URI (not the
        // specific subfolder URI). The paths cache has the exact content:// URI
        // for each subdirectory from previous reads.
        let uri = {
            let normalized_key = normalize_android_root_key(&payload.directory_path);

            // 1. Check the paths cache for an exact match (most accurate),
            //    then the LRU, where folders resolved by walking are kept.
            let exact_path_uri = {
                let paths = state.paths.lock().map_err(|_| {
                    "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                })?;
                paths
                    .get(&payload.directory_path)
                    .cloned()
                    .or_else(|| paths.get(&normalized_key).cloned())
            }
            .or_else(|| resolve_android_path_lru(state.inner(), &payload.directory_path));

            // The tree grant the folder belongs to: the payload's, or the
            // selected root that contains it.
            let tree = payload
                .directory_uri
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    let roots = state.roots.lock().ok()?;
                    roots
                        .get(&payload.directory_path)
                        .cloned()
                        .or_else(|| roots.get(&normalized_key).cloned())
                        .or_else(|| {
                            roots
                                .iter()
                                .filter(|(root_path, _)| {
                                    let norm_root = normalize_android_root_key(root_path);
                                    is_same_or_nested_path(root_path, &payload.directory_path)
                                        || is_same_or_nested_path(&norm_root, &payload.directory_path)
                                })
                                .max_by_key(|(root_path, _)| root_path.len())
                                .map(|(_, uri)| uri.clone())
                        })
                });
            let nested = tree.as_deref().filter(|tree| is_saf_tree_uri(tree)).and_then(|tree| {
                crate::filesystem::android_saf::android_relative_segments(&payload.directory_path, tree)
                    .map(|segments| (tree.to_string(), segments))
            });

            if exact_path_uri.is_some() {
                log::info!(
                    "[notia:directory_picker] read_android_directory uri resolved from paths cache"
                );
                exact_path_uri
            } else if let Some((tree, segments)) = nested {
                // 2. A folder inside the tree resolves to its own document,
                //    one shallow listing per level not known yet. Listing the
                //    tree instead would show the root's children as this
                //    folder's, and cache the root under the folder's path.
                match resolve_android_path_by_walking(state.inner(), &tree, &segments) {
                    Some(Some(uri)) => Some(uri),
                    Some(None) => return Err("La carpeta ya no existe en la biblioteca.".to_string()),
                    None => return Err("No se pudo leer el directorio Android.".to_string()),
                }
            } else if let Some(uri) = payload
                .directory_uri
                .clone()
                .filter(|value| !value.trim().is_empty())
            {
                // 3. The provided directoryUri is the folder itself (the tree
                //    root, or a document). Store it in the paths cache (not
                //    roots) unless this is a known library root.
                {
                    let mut paths = state.paths.lock().map_err(|_| {
                        "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                    })?;
                    paths.insert(payload.directory_path.clone(), uri.clone());
                    if !normalized_key.is_empty() {
                        paths.insert(normalized_key.clone(), uri.clone());
                    }
                }
                let is_known_root = {
                    let roots = state.roots.lock().map_err(|_| {
                        "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                    })?;
                    roots.contains_key(&payload.directory_path)
                        || roots.contains_key(&normalized_key)
                };
                if is_known_root {
                    let mut roots = state.roots.lock().map_err(|_| {
                        "No se pudo acceder a las carpetas Android seleccionadas.".to_string()
                    })?;
                    roots.insert(payload.directory_path.clone(), uri.clone());
                    roots.insert(normalized_key, uri.clone());
                }
                log::info!(
                    "[notia:directory_picker] read_android_directory uri resolved from payload"
                );
                Some(uri)
            } else {
                // 4. The folder is a selected root itself.
                tree
            }
        };

        let Some(uri) = uri else {
            log::warn!("[notia:directory_picker] read_android_directory uri resolution failed");
            return Err(
                "No se encontro la referencia Android de la carpeta seleccionada.".to_string(),
            );
        };

        log::info!("[notia:directory_picker] calling readDirectory plugin");

        // Call the Kotlin readDirectory command (shallow, non-recursive)
        let response = {
            let guard = state
                .handle
                .lock()
                .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
            let Some(handle) = guard.as_ref() else {
                return Err("El selector de carpetas no esta disponible.".to_string());
            };

            handle
                .run_mobile_plugin::<ReadTreeResponse>(
                    "readDirectory",
                    serde_json::json!({ "uri": uri }),
                )
                .map_err(|error| format!("No se pudo leer el directorio Android: {error}"))?
        };

        // Update the path cache from the shallow nodes we received.
        // Shallow nodes have id (content URI), name, and path which are
        // enough to populate the path cache for direct children.
        {
            let mut paths = state
                .paths
                .lock()
                .map_err(|_| "No se pudo actualizar la cache Android.".to_string())?;
            for node in &response.nodes {
                if let Some(path) = node.path.as_deref() {
                    if node.id.starts_with("content://") {
                        paths.insert(path.to_string(), node.id.clone());
                        let normalized = normalize_android_root_key(path);
                        if !normalized.is_empty() {
                            paths.insert(normalized, node.id.clone());
                        }
                    }
                }
            }
        }

        let mut nodes = response.nodes;
        sort_android_tree_nodes(&mut nodes);
        let node_count = nodes.len();
        timer.finish_with_meta(&format!("node_count={node_count}"));

        Ok(nodes)
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = state;
        let ReadAndroidTreePayload {
            directory_path,
            directory_uri,
        } = payload;
        let _ = (directory_path, directory_uri);
        Err("Solo disponible en Android.".to_string())
    }
}

/// Flat SAF listing of a granted tree for backend indexing. Entry paths are
/// `tree/<logical path>`; the caller validates and strips the tree prefix.
#[cfg(target_os = "android")]
pub(crate) fn read_android_flat_entries(
    state: &AndroidDirectoryPickerState,
    tree_uri: &str,
) -> Result<Vec<AndroidFlatFileEntry>, String> {
    validate_tree_context(Some(tree_uri))?;
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };
    handle
        .run_mobile_plugin::<ReadFlatFileListResponse>(
            "readFlatFileList",
            serde_json::json!({ "uri": tree_uri }),
        )
        .map(|response| response.files)
        .map_err(|error| format!("No se pudo leer la lista de archivos Android: {error}"))
}

#[cfg(target_os = "android")]
pub fn read_android_content_text(
    state: &AndroidDirectoryPickerState,
    content_uri: &str,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<ReadFileResponse>(
            "readFile",
            serde_json::json!({ "uri": content_uri }),
        )
        .map_err(|error| format!("No se pudo leer el archivo Android: {error}"))?;
    let Some(content) = response.content else {
        return Err("No se pudo leer el contenido del archivo Android.".to_string());
    };

    Ok(content)
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileBinaryResponse {
    name: Option<String>,
    base64: Option<String>,
}

/// Reads the raw bytes of a SAF document, Base64-decoded by Rust. Used for
/// binary documents (PDF/images) that must not go through text decoding.
#[cfg(target_os = "android")]
pub fn read_android_content_bytes(
    state: &AndroidDirectoryPickerState,
    content_uri: &str,
) -> Result<(String, Vec<u8>), String> {
    use base64::Engine as _;

    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<ReadFileBinaryResponse>(
            "readFileBinary",
            serde_json::json!({ "uri": content_uri }),
        )
        .map_err(|error| format!("No se pudo leer el archivo Android: {error}"))?;
    let Some(base64_payload) = response.base64 else {
        return Err("No se pudo leer el contenido del archivo Android.".to_string());
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_payload.trim())
        .map_err(|_| "El contenido binario del documento es inválido.".to_string())?;
    Ok((
        response
            .name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "documento".to_string()),
        bytes,
    ))
}

#[cfg(target_os = "android")]
pub fn write_android_content_text(
    state: &AndroidDirectoryPickerState,
    content_uri: &str,
    content: &str,
) -> Result<(), String> {
    write_android_content_bytes(state, content_uri, content.as_bytes())
}

#[cfg(target_os = "android")]
pub fn write_android_content_bytes(
    state: &AndroidDirectoryPickerState,
    content_uri: &str,
    data: &[u8],
) -> Result<(), String> {
    write_android_content_bytes_stamped(state, content_uri, data).map(|_| ())
}

/// [`write_android_content_bytes`], returning the size and modification
/// time (ms) the provider reports after the write, when it reports them.
#[cfg(target_os = "android")]
pub(crate) fn write_android_content_bytes_stamped(
    state: &AndroidDirectoryPickerState,
    content_uri: &str,
    data: &[u8],
) -> Result<Option<(u64, i64)>, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let payload_base64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(data)
    };

    let response = handle
        .run_mobile_plugin::<WriteFileResponse>(
            "writeFile",
            serde_json::json!({ "uri": content_uri, "base64": payload_base64 }),
        )
        .map_err(|error| format!("No se pudo escribir el archivo Android: {error}"))?;

    if !response.ok {
        return Err("No se pudo escribir el archivo Android.".to_string());
    }
    LIBRARY_WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(response.size.zip(response.last_modified))
}

#[cfg(target_os = "android")]
pub fn verify_android_document_under_tree(
    state: &AndroidDirectoryPickerState,
    tree_uri: &str,
    document_uri: &str,
    require_write: bool,
) -> Result<bool, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<VerifyDocumentResponse>(
            "verifyDocumentUnderTree",
            serde_json::json!({
                "treeUri": tree_uri,
                "documentUri": document_uri,
                "requireWrite": require_write,
            }),
        )
        .map_err(|error| format!("No se pudo verificar el documento Android: {error}"))?;
    Ok(response.ok)
}

#[cfg(target_os = "android")]
pub fn create_android_tree_entry(
    state: &AndroidDirectoryPickerState,
    parent_uri: &str,
    name: &str,
    entry_type: &str,
    content: Option<&str>,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<CreateEntryResponse>(
            "createEntry",
            serde_json::json!({
                "parentUri": parent_uri,
                "name": name,
                "entryType": entry_type,
                "content": content,
            }),
        )
        .map_err(|error| format!("No se pudo crear la entrada Android: {error}"))?;

    let Some(path) = response.path else {
        return Err("No se pudo crear la entrada Android.".to_string());
    };
    if path.trim().is_empty() {
        return Err("No se pudo crear la entrada Android.".to_string());
    }

    Ok(path)
}

#[cfg(target_os = "android")]
pub fn create_android_path_entry(
    state: &AndroidDirectoryPickerState,
    root_uri: &str,
    segments: &[String],
    entry_type: &str,
    content: Option<&str>,
) -> Result<String, String> {
    if segments.is_empty() {
        return Err("No se recibió una ruta Android válida.".to_string());
    }

    log::info!(
        "[notia:directory_picker] createPathEntry input root_uri_present={} root_uri_len={} segments={}",
        !root_uri.trim().is_empty(),
        root_uri.len(),
        segments.len()
    );

    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<CreateEntryResponse>(
            "createPathEntry",
            serde_json::json!({
                "rootUri": root_uri,
                "segments": segments,
                "entryType": entry_type,
                "content": content,
            }),
        )
        .map_err(|error| format!("No se pudo crear la ruta Android: {error}"))?;

    let Some(path) = response.path else {
        return Err("No se pudo crear la ruta Android.".to_string());
    };
    if path.trim().is_empty() {
        return Err("No se pudo crear la ruta Android.".to_string());
    }
    Ok(path)
}

#[cfg(target_os = "android")]
pub fn create_android_directory(
    state: &AndroidDirectoryPickerState,
    parent_uri: &str,
    dir_name: &str,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<CreateEntryResponse>(
            "createEntry",
            serde_json::json!({
                "parentUri": parent_uri,
                "name": dir_name,
                "entryType": "folder",
                "content": null,
            }),
        )
        .map_err(|error| format!("No se pudo crear el directorio Android: {error}"))?;

    let Some(path) = response.path else {
        return Err("No se pudo crear el directorio Android.".to_string());
    };

    Ok(path)
}

#[cfg(target_os = "android")]
pub fn delete_android_tree_entry(
    state: &AndroidDirectoryPickerState,
    entry_uri: &str,
) -> Result<(), String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<WriteFileResponse>(
            "deleteEntry",
            serde_json::json!({ "uri": entry_uri }),
        )
        .map_err(|error| format!("No se pudo eliminar la entrada Android: {error}"))?;

    if !response.ok {
        return Err("No se pudo eliminar la entrada Android.".to_string());
    }

    Ok(())
}

#[cfg(target_os = "android")]
pub fn rename_android_tree_entry(
    state: &AndroidDirectoryPickerState,
    entry_uri: &str,
    new_name: &str,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<EntryMutationResponse>(
            "renameEntry",
            serde_json::json!({ "uri": entry_uri, "newName": new_name }),
        )
        .map_err(|error| format!("No se pudo renombrar la entrada Android: {error}"))?;

    if !response.ok.unwrap_or(false) {
        return Err("No se pudo renombrar la entrada Android.".to_string());
    }

    let Some(path) = response.path else {
        return Err("No se pudo renombrar la entrada Android.".to_string());
    };
    if path.trim().is_empty() {
        return Err("No se pudo renombrar la entrada Android.".to_string());
    }

    Ok(path)
}

#[cfg(target_os = "android")]
pub fn copy_android_tree_entry(
    state: &AndroidDirectoryPickerState,
    source_uri: &str,
    target_parent_uri: &str,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<EntryMutationResponse>(
            "copyEntry",
            serde_json::json!({
                "sourceUri": source_uri,
                "targetParentUri": target_parent_uri,
            }),
        )
        .map_err(|error| format!("No se pudo copiar la entrada Android: {error}"))?;

    if !response.ok.unwrap_or(false) {
        return Err("No se pudo copiar la entrada Android.".to_string());
    }

    let Some(path) = response.path else {
        return Err("No se pudo copiar la entrada Android.".to_string());
    };
    if path.trim().is_empty() {
        return Err("No se pudo copiar la entrada Android.".to_string());
    }

    Ok(path)
}

#[cfg(target_os = "android")]
pub fn move_android_tree_entry(
    state: &AndroidDirectoryPickerState,
    source_uri: &str,
    source_parent_uri: &str,
    target_parent_uri: &str,
) -> Result<String, String> {
    let guard = state
        .handle
        .lock()
        .map_err(|_| "No se pudo acceder al selector de carpetas.".to_string())?;
    let Some(handle) = guard.as_ref() else {
        return Err("El selector de carpetas no esta disponible.".to_string());
    };

    let response = handle
        .run_mobile_plugin::<EntryMutationResponse>(
            "moveEntry",
            serde_json::json!({
                "sourceUri": source_uri,
                "sourceParentUri": source_parent_uri,
                "targetParentUri": target_parent_uri,
            }),
        )
        .map_err(|error| format!("No se pudo mover la entrada Android: {error}"))?;

    if !response.ok.unwrap_or(false) {
        return Err("No se pudo mover la entrada Android.".to_string());
    }

    let Some(path) = response.path else {
        return Err("No se pudo mover la entrada Android.".to_string());
    };
    if path.trim().is_empty() {
        return Err("No se pudo mover la entrada Android.".to_string());
    }

    Ok(path)
}

#[cfg(target_os = "android")]
pub fn resolve_android_tree_uri(
    state: &AndroidDirectoryPickerState,
    directory_path: &str,
    directory_uri: Option<&str>,
) -> Result<Option<String>, String> {
    let normalized_key = normalize_android_root_key(directory_path);
    if let Some(uri) = directory_uri
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    {
        let mut roots = state
            .roots
            .lock()
            .map_err(|_| "No se pudo acceder a las carpetas Android seleccionadas.".to_string())?;
        roots.insert(directory_path.to_string(), uri.clone());
        roots.insert(normalized_key.clone(), uri.clone());
        let mut paths = state
            .paths
            .lock()
            .map_err(|_| "No se pudo acceder a las carpetas Android seleccionadas.".to_string())?;
        paths.insert(directory_path.to_string(), uri.clone());
        paths.insert(normalized_key, uri.clone());
        return Ok(Some(uri));
    }

    // Try exact path match in the cached paths map first.
    let exact_path_uri = {
        let paths = state
            .paths
            .lock()
            .map_err(|_| "No se pudo acceder a las carpetas Android seleccionadas.".to_string())?;
        paths
            .get(directory_path)
            .cloned()
            .or_else(|| paths.get(&normalized_key).cloned())
    };
    if exact_path_uri.is_some() {
        return Ok(exact_path_uri);
    }

    // Try exact match in roots.
    let roots = state
        .roots
        .lock()
        .map_err(|_| "No se pudo acceder a las carpetas Android seleccionadas.".to_string())?;
    // Only exact entries are valid here. Returning a root URI for an unknown
    // nested path makes a missing file look like the library root and causes
    // reads/writes to target the wrong SAF document. Callers that have a root
    // context refresh the tree before retrying this exact lookup.
    let resolved = roots
        .get(directory_path)
        .cloned()
        .or_else(|| roots.get(&normalized_key).cloned());
    Ok(resolved)
}

#[cfg(all(test, target_os = "android"))]
mod tests {
    use super::{resolve_android_tree_uri, AndroidDirectoryPickerState};

    #[test]
    fn does_not_resolve_unknown_nested_paths_to_the_tree_root() {
        let state = AndroidDirectoryPickerState::unavailable();
        let root_path = "content://com.android.externalstorage.documents/tree/primary%3ADocuments";
        let root_uri = root_path.to_string();

        state
            .roots
            .lock()
            .expect("roots lock")
            .insert(root_path.to_string(), root_uri.clone());

        assert_eq!(
            resolve_android_tree_uri(&state, root_path, None).expect("root lookup"),
            Some(root_uri)
        );
        assert_eq!(
            resolve_android_tree_uri(
                &state,
                &format!("{root_path}/.notia/notiaConfig.json"),
                None
            )
            .expect("nested lookup"),
            None
        );
    }
}

#[cfg(test)]
mod path_scope_tests {
    use super::is_same_or_nested_path;

    #[test]
    fn prefix_matching_requires_a_segment_boundary() {
        assert!(is_same_or_nested_path("/library", "/library/notes"));
        assert!(is_same_or_nested_path("/library", "/library"));
        assert!(!is_same_or_nested_path("/library", "/library-old"));
    }
}

pub fn init() -> TauriPlugin<Wry> {
    PluginBuilder::new("notia-mobile")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                match api.register_android_plugin("com.gabriel.notia", "DirectoryPickerPlugin") {
                    Ok(handle) => {
                        app.manage(AndroidDirectoryPickerState::with_handle(handle));
                    }
                    Err(error) => {
                        log::error!(
                            "[notia:directory_picker] Android plugin not available, continuing without directory picker: {error}"
                        );
                        app.manage(AndroidDirectoryPickerState::unavailable());
                    }
                }
            }

            #[cfg(not(target_os = "android"))]
            {
                let _ = api;
                app.manage(AndroidDirectoryPickerState::empty());
            }

            Ok(())
        })
        .build()
}
