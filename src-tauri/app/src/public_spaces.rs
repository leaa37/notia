//! Public spaces of this device: one read-only HTTP server per space, each
//! on its own port, showing the published folders and notes with the
//! `#Publico` context as a GitBook-like site. Settings (device preferences
//! `publicSpaces`) say what each space publishes; the rules are in
//! `backend-core::public_spaces`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::public_spaces::site::{Site, SitePage};
use crate::backend::public_spaces::{
    free_port, in_items, is_covered, is_public_context, normalize_public_spaces, publishable_path, spaces_of, validate, ItemKind, PublicSpace,
    SpaceItem, PUBLIC_CONTEXT_TAG,
};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Manager};

const SECTION: &str = "publicSpaces";
/// A space reads its notes again after this long; edits show up then.
const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(10);
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const AUTOSTART_DELAY: Duration = Duration::from_secs(8);

/// What a space shows, read from the library.
pub(crate) struct Snapshot {
    built_at: Instant,
    generation: i64,
    space: PublicSpace,
    pub(crate) site: Site,
}

#[derive(Default)]
pub(crate) struct PublicSpacesState {
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    servers: Mutex<HashMap<String, server::Running>>,
    errors: Mutex<HashMap<String, String>>,
}

/// Whether the note at `path` is inside a folder that a published space
/// of this device publishes: it then carries `publicado`.
pub(crate) fn in_published_folder(app: &AppHandle, library_id: &str, path: &str) -> bool {
    crate::backend::public_spaces::in_published_folder(&stored_spaces(app), library_id, path)
}

/// Gives `publicado` to the notes of the folders the published spaces
/// publish; the ones that already have it keep their value.
fn stamp_published_folders(app: &AppHandle, spaces: &[PublicSpace]) {
    for space in spaces.iter().filter(|space| space.enabled) {
        for item in space.items.iter().filter(|item| item.kind == ItemKind::Folder) {
            let notes = crate::folder_contexts::indexed_inside(app, &space.library_id, &item.path);
            crate::folder_contexts::stamp(app, &space.library_id, &notes);
        }
    }
}

/// The spaces saved on this device.
pub(crate) fn stored_spaces(app: &AppHandle) -> Vec<PublicSpace> {
    spaces_of(&crate::device_preferences::section(app, SECTION))
}

fn space(app: &AppHandle, id: &str) -> Option<PublicSpace> {
    stored_spaces(app).into_iter().find(|space| space.id == id)
}

fn is_image(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Whether the note may be shown: its own context, or the one its folder
/// forces, is `#Publico`.
fn is_public_note(app: &AppHandle, library_id: &str, path: &str, source: &str) -> bool {
    is_public_context(crate::backend::library_graph::frontmatter_context(source).as_deref())
        || is_public_context(crate::folder_contexts::effective(app, library_id, path).as_deref())
}

fn build_snapshot(app: &AppHandle, space: &PublicSpace) -> Result<Snapshot, BackendError> {
    let (files, generation) = crate::library_inventory::inventory_files(app, &space.library_id)?;
    let mut pages = Vec::new();
    let mut images = Vec::new();
    crate::library_documents::with_documents(app, &space.library_id, |documents| {
        for path in files.iter().filter(|path| publishable_path(path).is_some() && in_items(&space.items, path)) {
            if path.to_lowercase().ends_with(".md") {
                let Some(source) = documents.read(path)? else { continue };
                if is_public_note(app, &space.library_id, path, &source) {
                    let modified_at = crate::library_inventory::file_modified_at(app, &space.library_id, path);
                    pages.push(SitePage { path: path.clone(), source, modified_at });
                }
            } else if is_image(path) {
                images.push(path.clone());
            }
        }
        Ok(())
    })?;
    Ok(Snapshot { built_at: Instant::now(), generation, site: Site::build(&space.name, &space.items, pages, images), space: space.clone() })
}

/// What the space `id` shows now, read again when it is older than
/// [`SNAPSHOT_LIFETIME`], its settings changed or the library was indexed again.
pub(crate) fn snapshot(app: &AppHandle, id: &str) -> Result<Arc<Snapshot>, BackendError> {
    let space = space(app, id).filter(|space| space.enabled).ok_or_else(|| BackendError::new(BackendErrorCode::NotFound, "El espacio no está publicado.", false))?;
    let state = app.state::<PublicSpacesState>();
    let generation = crate::library_inventory::inventory_files(app, &space.library_id).map(|(_, generation)| generation).unwrap_or_default();
    if let Some(cached) = state.snapshots.lock().ok().and_then(|cache| cache.get(id).cloned()) {
        if cached.space == space && cached.generation == generation && cached.built_at.elapsed() < SNAPSHOT_LIFETIME {
            return Ok(cached);
        }
    }
    let built = Arc::new(build_snapshot(app, &space)?);
    if let Ok(mut cache) = state.snapshots.lock() {
        cache.insert(id.to_string(), Arc::clone(&built));
    }
    Ok(built)
}

/// The bytes of a published image of the space.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn file_bytes(app: &AppHandle, snapshot: &Snapshot, url_path: &str) -> Option<(Vec<u8>, &'static str)> {
    let path = snapshot.site.file_for_url(url_path)?;
    let locator = crate::backend::DocumentLocatorDto::new(&snapshot.space.library_id, &path, None, None).ok()?;
    let bytes = crate::library_documents::with_documents(app, &snapshot.space.library_id, |documents| documents.adapter.read_binary_locator(&locator)).ok()?;
    let mime = match path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    Some((bytes, mime))
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Ports other services of this device use.
fn reserved_ports(app: &AppHandle) -> Vec<(u16, &'static str)> {
    let mut reserved = vec![(crate::connection::settings(app).port, "el servidor del host")];
    if let Some(port) = crate::device_preferences::section(app, "taskManagerPublication")["port"].as_u64().and_then(|port| u16::try_from(port).ok()) {
        reserved.push((port, "el tablero publicado"));
    }
    if let Some(port) = crate::webhooks::listening_port(app) {
        reserved.push((port, "el webhook"));
    }
    reserved
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemView {
    path: String,
    kind: ItemKind,
    name: String,
    /// `false` when it lost the `#Publico` context or no longer exists.
    public: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpaceView {
    id: String,
    name: String,
    library_id: String,
    port: u16,
    enabled: bool,
    items: Vec<ItemView>,
    listening: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    page_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicSpacesView {
    /// Public spaces run on Windows and Linux, not on a phone.
    available: bool,
    spaces: Vec<SpaceView>,
    suggested_port: u16,
}

fn item_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// Whether `item` has the `#Publico` context now.
fn item_is_public(app: &AppHandle, library_id: &str, item: &SpaceItem) -> bool {
    match item.kind {
        ItemKind::Folder => is_public_context(crate::folder_contexts::effective(app, library_id, &format!("{}/nota.md", item.path)).as_deref()),
        ItemKind::File => crate::library_session::read_library_text(app, library_id, &item.path)
            .is_ok_and(|source| is_public_note(app, library_id, &item.path, &source)),
    }
}

fn view(app: &AppHandle) -> PublicSpacesView {
    let spaces = stored_spaces(app);
    let state = app.state::<PublicSpacesState>();
    let errors = state.errors.lock().map(|errors| errors.clone()).unwrap_or_default();
    let listening = server::listening(app);
    let views = spaces
        .iter()
        .map(|space| {
            let is_listening = listening.contains(&space.id);
            let page_count = if is_listening { snapshot(app, &space.id).map(|snapshot| snapshot.site.page_count()).unwrap_or_default() } else { 0 };
            SpaceView {
                id: space.id.clone(),
                name: space.name.clone(),
                library_id: space.library_id.clone(),
                port: space.port,
                enabled: space.enabled,
                items: space
                    .items
                    .iter()
                    .map(|item| ItemView { path: item.path.clone(), kind: item.kind, name: item_name(&item.path), public: item_is_public(app, &space.library_id, item) })
                    .collect(),
                listening: is_listening,
                url: is_listening.then(|| server::url(space.port)),
                error: errors.get(&space.id).cloned(),
                page_count,
            }
        })
        .collect();
    let reserved: Vec<u16> = reserved_ports(app).into_iter().map(|(port, _)| port).collect();
    PublicSpacesView { available: server::AVAILABLE, suggested_port: free_port(&spaces, &reserved), spaces: views }
}

pub(crate) fn public_spaces_view(app: AppHandle) -> PublicSpacesView {
    view(&app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePayload {
    spaces: Value,
}

fn save(app: &AppHandle, payload: SavePayload) -> Result<PublicSpacesView, BackendError> {
    if !server::AVAILABLE {
        return Err(BackendError::new(BackendErrorCode::Unsupported, "Los espacios públicos se publican desde Windows.", false));
    }
    let section = normalize_public_spaces(&json!({ "spaces": payload.spaces }));
    let spaces = spaces_of(&section);
    validate(&spaces, &reserved_ports(app)).map_err(|message| BackendError::new(BackendErrorCode::InvalidInput, message, false))?;
    for space in &spaces {
        if crate::library_catalog::catalog_library(app, &space.library_id).is_none() {
            return Err(BackendError::invalid_input("La biblioteca del espacio ya no está en este equipo."));
        }
        if let Some(item) = space.items.iter().find(|item| !item_is_public(app, &space.library_id, item)) {
            return Err(BackendError::new(
                BackendErrorCode::InvalidInput,
                format!("«{}» no tiene el contexto {PUBLIC_CONTEXT_TAG}.", item_name(&item.path)),
                false,
            ));
        }
    }
    crate::device_preferences::update_section(app, SECTION, |_| Ok(section.clone()))?;
    stamp_published_folders(app, &spaces);
    if let Ok(mut cache) = app.state::<PublicSpacesState>().snapshots.lock() {
        cache.clear();
    }
    server::reconfigure(app);
    Ok(view(app))
}

pub(crate) fn public_spaces_save(app: AppHandle, payload: SavePayload) -> Result<PublicSpacesView, BackendError> {
    save(&app, payload)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CandidatesPayload {
    library_id: String,
    /// What the space already publishes (saved or not): it and what its
    /// folders hold are left out.
    #[serde(default)]
    items: Vec<SpaceItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Candidate {
    path: String,
    kind: ItemKind,
    name: String,
}

/// Folders and notes of the library with the `#Publico` context that a
/// space may still add: folders whose context (theirs or inherited) is it
/// and notes that have it, except what the space already publishes.
pub(crate) fn public_space_candidates(app: AppHandle, payload: CandidatesPayload) -> Result<Vec<Candidate>, BackendError> {
    let library_id = payload.library_id;
    let published = payload.items;
    let notes = crate::library_graph::library_notes(&app, &library_id)?;
    let folder_contexts = crate::folder_contexts::load(&app, &library_id);
    let mut folders: BTreeMap<String, String> = BTreeMap::new();
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for (path, context, source) in &notes {
        if publishable_path(path).is_none() {
            continue;
        }
        let mut folder = path.as_str();
        while let Some((parent, _)) = folder.rsplit_once('/') {
            if is_public_context(folder_contexts.effective(&format!("{parent}/nota.md"))) {
                folders.insert(parent.to_lowercase(), parent.to_string());
            }
            folder = parent;
        }
        if is_public_context(context.as_deref()) || is_public_note(&app, &library_id, path, source) {
            files.insert(path.to_lowercase(), path.clone());
        }
    }
    folders.retain(|_, path| !is_covered(&published, path));
    files.retain(|_, path| !is_covered(&published, path));
    Ok(folders
        .into_values()
        .map(|path| Candidate { name: item_name(&path), path, kind: ItemKind::Folder })
        .chain(files.into_values().map(|path| Candidate { name: item_name(&path), path, kind: ItemKind::File }))
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenPayload {
    id: String,
}

pub(crate) fn public_space_open(app: AppHandle, payload: OpenPayload) -> Result<(), BackendError> {
    let space = space(&app, &payload.id).ok_or_else(|| BackendError::invalid_input("El espacio no existe."))?;
    if !server::listening(&app).contains(&space.id) {
        return Err(BackendError::invalid_input("El espacio no está publicado."));
    }
    server::open_in_browser(&format!("http://127.0.0.1:{}/", space.port))
}

/// Plugin that publishes the saved spaces when Notia starts (not on a client
/// of a host, whose library is the host's).
pub(crate) fn init() -> crate::host::plugin::TauriPlugin<crate::host::Wry> {
    crate::host::plugin::Builder::new("public-spaces-autostart")
        .setup(|app, _api| {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let app = app.clone();
                let _ = std::thread::Builder::new().name("notia-public-spaces-autostart".into()).spawn(move || {
                    std::thread::sleep(AUTOSTART_DELAY);
                    if !crate::connection::is_client(&app) {
                        server::reconfigure(&app);
                        stamp_published_folders(&app, &stored_spaces(&app));
                    }
                });
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            let _ = app;
            Ok(())
        })
        .build()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn remember_error(app: &AppHandle, id: &str, error: Option<String>) {
    if let Ok(mut errors) = app.state::<PublicSpacesState>().errors.lock() {
        match error {
            Some(error) => errors.insert(id.to_string(), error),
            None => errors.remove(id),
        };
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod server;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod server {
    use crate::host::AppHandle;
    use crate::backend::BackendError;

    pub(super) const AVAILABLE: bool = false;

    pub(super) fn reconfigure(_app: &AppHandle) {}

    pub(super) fn listening(_app: &AppHandle) -> Vec<String> {
        Vec::new()
    }

    pub(super) fn url(port: u16) -> String {
        format!("http://127.0.0.1:{port}/")
    }

    pub(super) fn open_in_browser(_url: &str) -> Result<(), BackendError> {
        Err(BackendError::new(crate::backend::BackendErrorCode::Unsupported, "Los espacios públicos se abren desde Windows.", false))
    }
}
