//! The library's `.agent` workspace exposed to the interface: folder
//! structure, the visual copy of the default prompt, custom prompts and the
//! selected one, rules added by the agent, persistent memories, the agent's
//! own thoughts, the person's biography, their way of talking and the
//! people they mention. The content rules live in
//! `backend_core::agent_workspace`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use crate::host::{AppHandle, Manager};

use crate::backend::agent_knowledge::{AgentFiles, Organization};
use crate::backend::agent_workspace as workspace;
use crate::backend::{
    BackendError, BackendErrorCode,
};
use crate::library_documents::{inventory_paths, with_documents, Documents};
use crate::library_registry::LibraryBindingRegistry;
use crate::mobile_directory_picker::AndroidDirectoryPickerState;

const SELECTION_FILE: &str = "agent-prompt-selection.json";
const MAX_SELECTION_BYTES: u64 = 64 * 1024;

/// Libraries whose workspace was prepared in this session, and the lock
/// that serializes workspace writes and the selection file.
#[derive(Default)]
pub(crate) struct AgentWorkspaceState {
    prepared: Mutex<HashSet<String>>,
    lock: Mutex<()>,
}

impl AgentWorkspaceState {
    /// A state whose library counts as prepared, for tests without a
    /// library database.
    #[cfg(test)]
    pub(crate) fn prepared_for(library_id: &str) -> Self {
        Self { prepared: Mutex::new(HashSet::from([library_id.to_string()])), lock: Mutex::new(()) }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentLibraryPayload {
    library_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPromptPayload {
    library_id: String,
    file_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentItemsPayload {
    library_id: String,
    items: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPromptOption {
    pub(crate) file_name: String,
    pub(crate) name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPromptList {
    pub(crate) prompts: Vec<AgentPromptOption>,
    selected: String,
}

fn internal() -> BackendError {
    BackendError::new(BackendErrorCode::Internal, "No se pudo completar la operación del agente.", true)
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, BackendError> + Send + 'static,
) -> Result<T, BackendError> {
    crate::host::async_runtime::spawn_blocking(run).await.map_err(|_| internal())?
}

fn is_agent_text_file(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    [".md", ".markdown", ".txt"].iter().any(|extension| lowered.ends_with(extension))
}

/// Folders, rules (with personal facts moved to memory), memory, the visual
/// default prompt and the confidential context of every agent file.
fn prepare_workspace(app: &AppHandle, library_id: &str) -> Result<(), BackendError> {
    {
        let registry = app.state::<LibraryBindingRegistry>();
        let picker = app.state::<AndroidDirectoryPickerState>();
        for folder in workspace::AGENT_FOLDERS {
            let (parent, name) = folder.rsplit_once('/').unwrap_or(("", folder));
            crate::filesystem::commands::ensure_library_folder(
                registry.inner(),
                picker.inner(),
                library_id,
                parent,
                name,
            )?;
        }
    }
    with_documents(app, library_id, |documents| {
        let rules = documents.read(workspace::RULES_PATH)?;
        let (rules_body, misclassified) =
            workspace::migrate_misclassified_rules(rules.as_deref().map(workspace::document_body).unwrap_or(""));
        documents.write(
            workspace::RULES_PATH,
            rules.as_deref(),
            &workspace::with_system_context(&rules_body),
        )?;

        let memory = documents.read(workspace::MEMORY_PATH)?;
        let mut memories = workspace::parse_memory_items(memory.as_deref().map(workspace::document_body).unwrap_or(""));
        memories.extend(misclassified);
        let memory_content = workspace::with_system_context(&workspace::render_memories(&workspace::merge_memories(memories)));
        let unchanged_memory = memory.as_deref().is_some_and(|current| {
            workspace::parse_memory_items(workspace::document_body(current))
                == workspace::parse_memory_items(workspace::document_body(&memory_content))
                && workspace::with_system_context(current) == *current
        });
        if !unchanged_memory {
            documents.write(workspace::MEMORY_PATH, memory.as_deref(), &memory_content)?;
        }
        write_missing_lists(documents)?;
        // The agents of the AI tasks start with their default prompts; the
        // person may edit them, so they are only written when missing.
        for phase in crate::backend::ai_tasks::AiTaskPhase::ALL {
            let path = format!("{}/{}", workspace::PROMPTS_DIRECTORY, phase.prompt_file());
            if documents.read(&path)?.is_none() {
                documents.write(&path, None, &workspace::with_confidential_context(phase.default_prompt()))?;
            }
        }

        let default_prompt = documents.read(workspace::DEFAULT_PROMPT_PATH)?;
        documents.write(
            workspace::DEFAULT_PROMPT_PATH,
            default_prompt.as_deref(),
            crate::backend::DEFAULT_AGENT_PROMPT,
        )?;
        Ok(())
    })?;
    let files = inventory_paths(app, library_id, workspace::AGENT_DIRECTORY)?;
    with_documents(app, library_id, |documents| {
        for path in files.iter().filter(|path| is_agent_text_file(path)) {
            if path.eq_ignore_ascii_case(workspace::DEFAULT_PROMPT_PATH) {
                continue;
            }
            if let Some(current) = documents.read(path)? {
                documents.write(path, Some(&current), &workspace::with_agent_context(path, &current))?;
            }
        }
        Ok(())
    })
}

/// Prepares the workspace once per library and session.
pub(crate) fn ensure_workspace(app: &AppHandle, library_id: &str) -> Result<(), BackendError> {
    let state = app.state::<AgentWorkspaceState>();
    let _guard = state.lock.lock().map_err(|_| internal())?;
    if state.prepared.lock().map_err(|_| internal())?.contains(library_id) {
        return Ok(());
    }
    prepare_workspace(app, library_id)?;
    state.prepared.lock().map_err(|_| internal())?.insert(library_id.to_string());
    Ok(())
}

/// Forgets the prepared mark so the next access rebuilds the workspace.
pub(crate) fn forget_library(app: &AppHandle, library_id: &str) {
    if let Ok(mut prepared) = app.state::<AgentWorkspaceState>().prepared.lock() {
        prepared.remove(library_id);
    }
}

fn selection_file(app: &AppHandle) -> Result<PathBuf, BackendError> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(SELECTION_FILE))
        .map_err(|_| BackendError::new(BackendErrorCode::Storage, "No se pudo acceder a la selección de prompts.", true))
}

fn read_selections(app: &AppHandle) -> HashMap<String, String> {
    let Ok(path) = selection_file(app) else {
        return HashMap::new();
    };
    if std::fs::metadata(&path).map(|metadata| metadata.len() > MAX_SELECTION_BYTES).unwrap_or(true) {
        return HashMap::new();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Prompt file selected for the library on this device.
pub(crate) fn selected_prompt(app: &AppHandle, library_id: &str) -> String {
    read_selections(app)
        .get(library_id)
        .map(|name| workspace::normalize_prompt_file_name(name))
        .unwrap_or_else(|| workspace::DEFAULT_PROMPT_FILE.to_string())
}

fn save_selection(app: &AppHandle, library_id: &str, file_name: &str) -> Result<(), BackendError> {
    let mut selections = read_selections(app);
    selections.insert(library_id.to_string(), workspace::normalize_prompt_file_name(file_name));
    let path = selection_file(app)?;
    let storage = || BackendError::new(BackendErrorCode::Storage, "No se pudo guardar la selección de prompt.", true);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| storage())?;
    }
    let text = serde_json::to_string_pretty(&selections).map_err(|_| storage())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|_| storage())?;
    std::fs::rename(&temporary, &path).map_err(|_| storage())
}

pub(crate) fn list_prompts(app: &AppHandle, library_id: &str) -> Result<AgentPromptList, BackendError> {
    ensure_workspace(app, library_id)?;
    let prefix = format!("{}/", workspace::PROMPTS_DIRECTORY);
    let names = inventory_paths(app, library_id, workspace::PROMPTS_DIRECTORY)?
        .into_iter()
        .filter_map(|path| path.strip_prefix(&prefix).map(str::to_string))
        .filter(|name| !name.contains('/') && name.to_ascii_lowercase().ends_with(".md"));
    let prompts = workspace::prompt_file_names(names)
        .into_iter()
        .map(|file_name| AgentPromptOption {
            name: file_name[..file_name.len() - 3].to_string(),
            file_name,
        })
        .collect::<Vec<_>>();
    let selected = selected_prompt(app, library_id);
    let selected = if prompts.iter().any(|prompt| prompt.file_name == selected) {
        selected
    } else {
        workspace::DEFAULT_PROMPT_FILE.to_string()
    };
    Ok(AgentPromptList { prompts, selected })
}

fn validate_items(items: &[String]) -> Result<(), BackendError> {
    if items.len() > workspace::MAX_MEMORIES || items.iter().any(|item| item.chars().count() > workspace::MAX_RULE_CHARS) {
        return Err(BackendError::invalid_input("La lista del agente no es válida o supera el límite."));
    }
    Ok(())
}

fn with_workspace_lock<T>(app: &AppHandle, run: impl FnOnce() -> Result<T, BackendError>) -> Result<T, BackendError> {
    let state = app.state::<AgentWorkspaceState>();
    let _guard = state.lock.lock().map_err(|_| internal())?;
    run()
}

pub(crate) async fn backend_agent_prompts(
    app: AppHandle,
    payload: AgentLibraryPayload,
) -> Result<AgentPromptList, BackendError> {
    blocking(move || list_prompts(&app, &payload.library_id)).await
}

pub(crate) async fn backend_select_agent_prompt(
    app: AppHandle,
    payload: AgentPromptPayload,
) -> Result<String, BackendError> {
    blocking(move || {
        app.state::<LibraryBindingRegistry>().lookup(&payload.library_id)?;
        with_workspace_lock(&app, || save_selection(&app, &payload.library_id, &payload.file_name))?;
        Ok(selected_prompt(&app, &payload.library_id))
    })
    .await
}

/// Replaces the memories (deduplicated and capped).
pub(crate) fn save_memories(app: &AppHandle, library_id: &str, items: Vec<String>) -> Result<Vec<String>, BackendError> {
    validate_items(&items)?;
    ensure_workspace(app, library_id)?;
    let memories = workspace::merge_memories(items);
    with_workspace_lock(app, || {
        with_documents(app, library_id, |documents| {
            let current = documents.read(workspace::MEMORY_PATH)?;
            documents.write(
                workspace::MEMORY_PATH,
                current.as_deref(),
                &workspace::with_system_context(&workspace::render_memories(&memories)),
            )
        })
    })?;
    Ok(memories)
}

pub(crate) async fn backend_save_agent_memories(app: AppHandle, payload: AgentItemsPayload) -> Result<Vec<String>, BackendError> {
    blocking(move || {
        let saved = save_memories(&app, &payload.library_id, payload.items)?;
        if !saved.is_empty() {
            crate::agent_knowledge::schedule_organization(&app, &payload.library_id);
        }
        Ok(saved)
    })
    .await
}

fn read_items(documents: &Documents<'_>, path: &str) -> Result<(Option<String>, Vec<String>), BackendError> {
    let current = documents.read(path)?;
    let items = current
        .as_deref()
        .map(|content| workspace::parse_memory_items(workspace::document_body(content)))
        .unwrap_or_default();
    Ok((current, items))
}

/// Writes the empty thoughts, biography, talk, people and improvements
/// files that are missing. Returns the paths it wrote.
fn write_missing_lists(documents: &Documents<'_>) -> Result<Vec<&'static str>, BackendError> {
    let empty = [
        (workspace::THOUGHTS_PATH, workspace::render_thoughts(&[])),
        (workspace::BIOGRAPHY_PATH, workspace::Biography::default().render()),
        (workspace::TALK.path, workspace::TALK.render(&[])),
        (workspace::PEOPLE.path, workspace::PEOPLE.render(&[])),
        (workspace::IMPROVE_PATH, workspace::render_improvements(&[])),
    ];
    let mut written = Vec::new();
    for (path, body) in empty {
        if documents.read(path)?.is_none() {
            documents.write(path, None, &workspace::with_system_context(&body))?;
            written.push(path);
        }
    }
    if crate::agent_scripts::ensure_readme(documents)? {
        written.push(crate::backend::agent_scripts::README_PATH);
    }
    Ok(written)
}

fn read_improvements(documents: &Documents<'_>) -> Result<(Option<String>, Vec<String>), BackendError> {
    let current = documents.read(workspace::IMPROVE_PATH)?;
    let entries = current
        .as_deref()
        .map(|content| workspace::parse_improvements(workspace::document_body(content)))
        .unwrap_or_default();
    Ok((current, entries))
}

/// Adds an error, missing tool or improvement to `improve.md`, dated with
/// the local time. The same kind and description is not added twice, and
/// the oldest entries go when the file is full. Nothing reads it into a
/// prompt, so it is not organized with the other agent files. Returns
/// whether the file changed.
pub(crate) fn append_improvement(
    app: &AppHandle,
    library_id: &str,
    kind: workspace::ImprovementKind,
    text: &str,
) -> Result<bool, BackendError> {
    let entry = workspace::improvement_entry(&crate::local_time::thought_stamp(), kind, text).ok_or_else(|| {
        BackendError::invalid_input(format!(
            "Describí la mejora en una o dos oraciones, de hasta {} caracteres.",
            workspace::MAX_IMPROVEMENT_CHARS
        ))
    })?;
    ensure_workspace(app, library_id)?;
    with_workspace_lock(app, || {
        with_documents(app, library_id, |documents| {
            let (current, entries) = read_improvements(documents)?;
            let Some(next) = workspace::with_improvement(&entries, entry.clone()) else {
                return Ok(false);
            };
            let body = workspace::with_system_context(&workspace::render_improvements(&next));
            documents.write(workspace::IMPROVE_PATH, current.as_deref(), &body)?;
            Ok(true)
        })
    })
}

/// The entries of `improve.md`, oldest first: only the reflection reads
/// them, so it does not note one again.
pub(crate) fn improvements(app: &AppHandle, library_id: &str) -> Result<Vec<String>, BackendError> {
    ensure_workspace(app, library_id)?;
    with_documents(app, library_id, |documents| read_improvements(documents).map(|(_, entries)| entries))
}

/// Creates `thoughts.md`, `biography.md`, `talk.md`, `people.md`,
/// `improve.md` and the guide of `.agent/scripts` again when they are
/// missing (deleted or never made). Unlike the session preparation, it
/// looks at the files every time. Returns the paths it wrote.
pub(crate) fn ensure_agent_list_files(app: &AppHandle, library_id: &str) -> Result<Vec<&'static str>, BackendError> {
    with_workspace_lock(app, || with_documents(app, library_id, write_missing_lists))
}

fn read_biography(documents: &Documents<'_>) -> Result<(Option<String>, workspace::Biography), BackendError> {
    let current = documents.read(workspace::BIOGRAPHY_PATH)?;
    let biography = workspace::Biography::parse(current.as_deref().map(workspace::document_body).unwrap_or(""));
    Ok((current, biography))
}

fn append_biography_locked(app: &AppHandle, library_id: &str, fact: &str) -> Result<Appended, BackendError> {
    with_documents(app, library_id, |documents| {
        let (current, biography) = read_biography(documents)?;
        let next = match biography.with_note(fact) {
            workspace::BiographyAppend::Added(next) => next,
            workspace::BiographyAppend::Unchanged => return Ok(Appended::Changed(false)),
            workspace::BiographyAppend::Full => return Ok(Appended::Full),
        };
        documents.write(workspace::BIOGRAPHY_PATH, current.as_deref(), &workspace::with_system_context(&next.render()))?;
        Ok(Appended::Changed(true))
    })
}

/// The text of every agent file of `.agent/memory`, as read, so a write
/// can tell whether it changes it.
struct AgentFileTexts {
    rules: Option<String>,
    memory: Option<String>,
    thoughts: Option<String>,
    talk: Option<String>,
    people: Option<String>,
    biography: Option<String>,
}

fn read_agent_files(documents: &Documents<'_>) -> Result<(AgentFileTexts, AgentFiles), BackendError> {
    let texts = AgentFileTexts {
        rules: documents.read(workspace::RULES_PATH)?,
        memory: documents.read(workspace::MEMORY_PATH)?,
        thoughts: documents.read(workspace::THOUGHTS_PATH)?,
        talk: documents.read(workspace::TALK.path)?,
        people: documents.read(workspace::PEOPLE.path)?,
        biography: documents.read(workspace::BIOGRAPHY_PATH)?,
    };
    let body = |text: &Option<String>| text.as_deref().map(workspace::document_body).unwrap_or("").to_string();
    let files = AgentFiles {
        rules: workspace::ia_rules(&body(&texts.rules)),
        memories: workspace::parse_memory_items(&body(&texts.memory)),
        thoughts: workspace::parse_memory_items(&body(&texts.thoughts)),
        talk: workspace::parse_memory_items(&body(&texts.talk)),
        people: workspace::parse_memory_items(&body(&texts.people)),
        biography: workspace::Biography::parse(&body(&texts.biography)),
    };
    Ok((texts, files))
}

/// Every agent file of `.agent/memory`: the list files item by item (the
/// rules without the managed default block) and the biography.
pub(crate) fn agent_files(app: &AppHandle, library_id: &str) -> Result<AgentFiles, BackendError> {
    ensure_workspace(app, library_id)?;
    with_documents(app, library_id, |documents| read_agent_files(documents).map(|(_, files)| files))
}

/// Writes what an organization changed, each file in its own form, only
/// when no agent file changed since `expected` was read, so an item saved
/// meanwhile is never lost (the next organization reads it). A file the
/// organization left as it was is not written. Returns whether it wrote.
pub(crate) fn replace_agent_files_if_unchanged(
    app: &AppHandle,
    library_id: &str,
    expected: &AgentFiles,
    organization: &Organization,
) -> Result<bool, BackendError> {
    with_workspace_lock(app, || {
        with_documents(app, library_id, |documents| {
            let (texts, saved) = read_agent_files(documents)?;
            if saved != *expected {
                return Ok(false);
            }
            let mut wrote = false;
            let mut write = |path: &str, current: &Option<String>, body: String| -> Result<(), BackendError> {
                let next = workspace::with_system_context(&body);
                if current.as_deref() != Some(next.as_str()) {
                    documents.write(path, current.as_deref(), &next)?;
                    wrote = true;
                }
                Ok(())
            };
            if let Some(lists) = &organization.lists {
                if lists.rules != saved.rules {
                    let body = texts.rules.as_deref().map(workspace::document_body).unwrap_or("");
                    write(workspace::RULES_PATH, &texts.rules, workspace::replace_ia_rules(body, &lists.rules))?;
                }
                if lists.memories != saved.memories {
                    write(workspace::MEMORY_PATH, &texts.memory, workspace::render_memories(&lists.memories))?;
                }
                if lists.thoughts != saved.thoughts {
                    write(workspace::THOUGHTS_PATH, &texts.thoughts, workspace::render_thoughts(&lists.thoughts))?;
                }
                if lists.talk != saved.talk {
                    write(workspace::TALK.path, &texts.talk, workspace::TALK.render(&lists.talk))?;
                }
                if lists.people != saved.people {
                    write(workspace::PEOPLE.path, &texts.people, workspace::PEOPLE.render(&lists.people))?;
                }
            }
            if let Some(story) = &organization.biography {
                let told = workspace::Biography { story: story.clone(), notes: Vec::new() };
                write(workspace::BIOGRAPHY_PATH, &texts.biography, told.render())?;
            }
            Ok(wrote)
        })
    })
}

/// Rewrites each agent file whose text is not in its canonical form (a
/// hand edit, an old version, stray markers), keeping every item: the rules
/// with the current default block, the memory, thoughts, talk and people
/// as clean deduplicated lists, and the biography's story and facts to
/// tell, each with its version marker and the confidential context. A line
/// of `people.md` takes its `Nombre — quién es` form when it has one.
/// Returns how many files it rewrote.
pub(crate) fn normalize_agent_files(app: &AppHandle, library_id: &str) -> Result<usize, BackendError> {
    ensure_workspace(app, library_id)?;
    let canonical = |path: &str, body: &str| match path {
        workspace::RULES_PATH => workspace::canonical_rules(body),
        workspace::MEMORY_PATH => workspace::render_memories(&workspace::merge_memories(workspace::parse_memory_items(body))),
        workspace::THOUGHTS_PATH => workspace::render_thoughts(&workspace::parse_memory_items(body)),
        workspace::BIOGRAPHY_PATH => workspace::Biography::parse(body).render(),
        workspace::PEOPLE_PATH => workspace::PEOPLE.render(
            &workspace::parse_memory_items(body)
                .into_iter()
                .map(|line| workspace::parse_person_entry(&line).unwrap_or(line))
                .collect::<Vec<_>>(),
        ),
        _ => workspace::TALK.render(&workspace::parse_memory_items(body)),
    };
    let comparable = |text: &str| text.replace("\r\n", "\n").trim().to_string();
    with_workspace_lock(app, || {
        with_documents(app, library_id, |documents| {
            let mut rewritten = 0;
            for path in [
                workspace::RULES_PATH,
                workspace::MEMORY_PATH,
                workspace::THOUGHTS_PATH,
                workspace::BIOGRAPHY_PATH,
                workspace::TALK.path,
                workspace::PEOPLE.path,
            ] {
                let Some(current) = documents.read(path)? else { continue };
                let next = workspace::with_system_context(&canonical(path, workspace::document_body(&current)));
                if comparable(&next) != comparable(&current) {
                    documents.write(path, Some(&current), &next)?;
                    rewritten += 1;
                }
            }
            Ok(rewritten)
        })
    })
}

/// Result of adding one item to a bounded agent file.
enum Appended {
    Changed(bool),
    /// The file has no room: it must be rewritten first.
    Full,
}

fn append_memory_locked(app: &AppHandle, library_id: &str, value: &str) -> Result<Appended, BackendError> {
    with_documents(app, library_id, |documents| {
        let (current, mut memories) = read_items(documents, workspace::MEMORY_PATH)?;
        if memories.iter().any(|memory| memory.eq_ignore_ascii_case(value)) {
            return Ok(Appended::Changed(false));
        }
        if !workspace::MEMORY_LIMIT.fits_one_more(&memories, value) {
            return Ok(Appended::Full);
        }
        memories.push(value.to_string());
        documents.write(
            workspace::MEMORY_PATH,
            current.as_deref(),
            &workspace::with_system_context(&workspace::render_memories(&memories)),
        )?;
        Ok(Appended::Changed(true))
    })
}

/// Adds a dated thought. A full file drops its oldest thoughts down to the
/// target size, so the new note is always kept; the organization after the
/// save still merges and discards what no longer matters.
fn append_thought_locked(app: &AppHandle, library_id: &str, stamped: &str) -> Result<bool, BackendError> {
    with_documents(app, library_id, |documents| {
        let (current, thoughts) = read_items(documents, workspace::THOUGHTS_PATH)?;
        let mut next = workspace::with_thought(&thoughts, stamped.to_string());
        if !workspace::THOUGHTS_LIMIT.fits(&next) {
            let (kept, dropped) = workspace::without_oldest(next, workspace::THOUGHTS_TARGET);
            log::info!("[notia:memory] pensamientos llenos: se quitaron los {dropped} más antiguos");
            next = kept;
        }
        documents.write(
            workspace::THOUGHTS_PATH,
            current.as_deref(),
            &workspace::with_system_context(&workspace::render_thoughts(&next)),
        )?;
        Ok(true)
    })
}

/// Adds an item under the workspace lock. When the file is full, the model
/// rewrites it (`rewrite`, run without the lock because it calls the model)
/// and the item is added once more; a file still full is an error, so
/// nothing is dropped silently.
fn append_bounded(
    app: &AppHandle,
    append: impl Fn() -> Result<Appended, BackendError>,
    rewrite: impl FnOnce() -> Result<bool, BackendError>,
    full: &str,
) -> Result<bool, BackendError> {
    if let Appended::Changed(changed) = with_workspace_lock(app, &append)? {
        return Ok(changed);
    }
    if let Err(error) = rewrite() {
        log::error!("[notia:memory] no se pudo reescribir un archivo lleno del agente: {:?}", error.code);
    }
    match with_workspace_lock(app, &append)? {
        Appended::Changed(changed) => Ok(changed),
        Appended::Full => Err(BackendError::invalid_input(full)),
    }
}

fn append_list_locked(
    app: &AppHandle,
    library_id: &str,
    file: workspace::AgentListFile,
    value: &str,
) -> Result<Appended, BackendError> {
    with_documents(app, library_id, |documents| {
        let (current, mut items) = read_items(documents, file.path)?;
        if items.iter().any(|item| item.eq_ignore_ascii_case(value)) {
            return Ok(Appended::Changed(false));
        }
        if !file.limit.fits_one_more(&items, value) {
            return Ok(Appended::Full);
        }
        items.push(value.to_string());
        documents.write(file.path, current.as_deref(), &workspace::with_system_context(&file.render(&items)))?;
        Ok(Appended::Changed(true))
    })
}

fn append_rule_locked(app: &AppHandle, library_id: &str, rule: &str) -> Result<Appended, BackendError> {
    with_documents(app, library_id, |documents| {
        let current = documents.read(workspace::RULES_PATH)?;
        let body = match workspace::append_rule(current.as_deref().map(workspace::document_body).unwrap_or(""), rule) {
            workspace::RuleAppend::Added(body) => body,
            workspace::RuleAppend::Unchanged => return Ok(Appended::Changed(false)),
            workspace::RuleAppend::Full => return Ok(Appended::Full),
        };
        documents.write(workspace::RULES_PATH, current.as_deref(), &workspace::with_system_context(&body))?;
        Ok(Appended::Changed(true))
    })
}

/// Persists a rule, memory, thought, part of the biography, trait of the
/// person's way of talking or person requested by the agent tool (a person
/// arrives as its `Nombre — quién es` line), with the same
/// format the interface reads. A thought is dated here. A full file is
/// organized by the model, together with the other agent files, before
/// adding. Returns whether the file changed.
pub(crate) fn append_agent_item(app: &AppHandle, library_id: &str, kind: AgentItem, value: &str) -> Result<bool, BackendError> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() || value.chars().count() > workspace::MAX_RULE_CHARS {
        return Err(BackendError::invalid_input("El contenido de memoria no es válido o supera el límite."));
    }
    ensure_workspace(app, library_id)?;
    match kind {
        AgentItem::Rule => append_bounded(
            app,
            || append_rule_locked(app, library_id, &value),
            || crate::agent_knowledge::organize_now(app, library_id),
            "Las reglas del agente están llenas y no se pudieron reorganizar. Probá de nuevo más tarde.",
        ),
        AgentItem::Memory => append_bounded(
            app,
            || append_memory_locked(app, library_id, &value),
            || crate::agent_knowledge::organize_now(app, library_id),
            "La memoria del agente está llena y no se pudo reorganizar. Probá de nuevo más tarde.",
        ),
        AgentItem::Thought => {
            let stamped = workspace::stamp_thought(&crate::local_time::thought_stamp(), &value).ok_or_else(|| {
                BackendError::invalid_input(format!(
                    "Un pensamiento debe ser una oración breve de hasta {} caracteres.",
                    workspace::MAX_THOUGHT_CHARS
                ))
            })?;
            with_workspace_lock(app, || append_thought_locked(app, library_id, &stamped))
        }
        AgentItem::Biography => {
            if value.chars().count() > workspace::MAX_BIOGRAPHY_NOTE_CHARS {
                return Err(BackendError::invalid_input(format!(
                    "Cada dato de la biografía debe ser una oración breve de hasta {} caracteres.",
                    workspace::MAX_BIOGRAPHY_NOTE_CHARS
                )));
            }
            append_bounded(
                app,
                || append_biography_locked(app, library_id, &value),
                || crate::agent_knowledge::organize_now(app, library_id),
                "La biografía tiene demasiados datos sin contar y no se pudo escribir. Probá de nuevo más tarde.",
            )
        }
        AgentItem::Talk => {
            if value.chars().count() > workspace::TALK.max_item_chars {
                return Err(BackendError::invalid_input(format!(
                    "Cada rasgo debe ser una oración breve de hasta {} caracteres.",
                    workspace::TALK.max_item_chars
                )));
            }
            append_bounded(
                app,
                || append_list_locked(app, library_id, workspace::TALK, &value),
                || crate::agent_knowledge::organize_now(app, library_id),
                "El archivo del agente está lleno y no se pudo reorganizar. Probá de nuevo más tarde.",
            )
        }
        AgentItem::Person => {
            let entry = workspace::parse_person_entry(&value).ok_or_else(|| {
                BackendError::invalid_input(format!(
                    "Cada persona necesita su nombre (hasta {} caracteres) y quién es, en hasta {} caracteres.",
                    workspace::MAX_PERSON_NAME_CHARS,
                    workspace::PEOPLE.max_item_chars
                ))
            })?;
            append_bounded(
                app,
                || append_list_locked(app, library_id, workspace::PEOPLE, &entry),
                || crate::agent_knowledge::organize_now(app, library_id),
                "El archivo de personas está lleno y no se pudo reorganizar. Probá de nuevo más tarde.",
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AgentItem {
    Rule,
    Memory,
    Thought,
    Biography,
    Talk,
    /// A `Nombre — quién es` line of `people.md`.
    Person,
}
