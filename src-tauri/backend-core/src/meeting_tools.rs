//! Agent tools over the open Meeting: read its state and Notas IA, replace
//! or extend those notes, change the library context its AI reads and fix
//! misheard words in the transcript. This module holds the tool names and
//! the argument parsing; the record applies the changes (`meeting.rs`) and
//! the app executes them where the meeting state lives.

use serde_json::Value;

use crate::error::BackendError;
use crate::library_config::normalize_context_tag;
use crate::meeting::MeetingRecord;
use crate::meeting_ai::{
    clip, normalized_folder, parse_clock, MeetingContextChoice, MeetingNoteTask, MeetingNoteTopic, MAX_ITEM_CHARS,
    MAX_LIST_ITEMS, MAX_NOTE_TASKS, MAX_OBJECTIVE_CHARS, MAX_TASK_FIELD_CHARS, MAX_TOPICS, MAX_TOPIC_TITLE_CHARS,
    UNTAGGED,
};

pub const MEETING_READ_TOOLS: [&str; 1] = ["get_meeting_overview"];
pub const MEETING_WRITE_TOOLS: [&str; 4] =
    ["update_meeting_notes", "add_meeting_notes", "set_meeting_ai_context", "correct_meeting_transcript"];

/// Replacements of one `correct_meeting_transcript` call.
pub const MAX_TRANSCRIPT_REPLACEMENTS: usize = 20;
const MAX_REPLACEMENT_CHARS: usize = 120;

pub fn is_meeting_tool(name: &str) -> bool {
    MEETING_READ_TOOLS.contains(&name) || is_meeting_write_tool(name)
}

pub fn is_meeting_write_tool(name: &str) -> bool {
    MEETING_WRITE_TOOLS.contains(&name)
}

fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::invalid_input(message)
}

fn text(value: &Value, key: &str, max: usize) -> String {
    value.get(key).and_then(Value::as_str).map(|text| clip(text, max)).unwrap_or_default()
}

fn items(value: &Value, key: &str, max_items: usize) -> Option<Vec<String>> {
    value.get(key).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| clip(item, MAX_ITEM_CHARS))
            .filter(|item| !item.is_empty())
            .take(max_items)
            .collect()
    })
}

fn parsed_topic(topic: &Value) -> Option<MeetingNoteTopic> {
    let title = text(topic, "title", MAX_TOPIC_TITLE_CHARS);
    let at_ms = topic.get("minute").and_then(Value::as_str).and_then(parse_clock).unwrap_or(0);
    let items = items(topic, "items", crate::meeting_ai::MAX_TOPIC_ITEMS).unwrap_or_default();
    (!title.is_empty()).then_some(MeetingNoteTopic { title, at_ms, items })
}

/// A task of the arguments, without an id: the record assigns one, keeping
/// the id and the sent mark of a task whose text does not change.
fn parsed_task(task: &Value) -> Option<MeetingNoteTask> {
    let (task_text, owner, due) = match task {
        Value::String(task_text) => (clip(task_text, MAX_ITEM_CHARS), String::new(), String::new()),
        Value::Object(_) => (
            text(task, "text", MAX_ITEM_CHARS),
            text(task, "owner", MAX_TASK_FIELD_CHARS),
            text(task, "due", MAX_TASK_FIELD_CHARS),
        ),
        _ => return None,
    };
    (!task_text.is_empty()).then(|| MeetingNoteTask { id: String::new(), sent: false, text: task_text, owner, due })
}

/// `update_meeting_notes`: each present section replaces the saved one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NotesUpdate {
    pub objective: Option<String>,
    pub decisions: Option<Vec<String>>,
    pub open_questions: Option<Vec<String>>,
    pub topics: Option<Vec<MeetingNoteTopic>>,
    pub tasks: Option<Vec<MeetingNoteTask>>,
}

pub fn parse_notes_update(arguments: &Value) -> Result<NotesUpdate, BackendError> {
    let update = NotesUpdate {
        objective: arguments.get("objective").and_then(Value::as_str).map(|objective| clip(objective, MAX_OBJECTIVE_CHARS)),
        decisions: items(arguments, "decisions", MAX_LIST_ITEMS),
        open_questions: items(arguments, "openQuestions", MAX_LIST_ITEMS),
        topics: arguments
            .get("topics")
            .and_then(Value::as_array)
            .map(|topics| topics.iter().filter_map(parsed_topic).take(MAX_TOPICS).collect()),
        tasks: arguments
            .get("tasks")
            .and_then(Value::as_array)
            .map(|tasks| tasks.iter().filter_map(parsed_task).take(MAX_NOTE_TASKS).collect()),
    };
    if update == NotesUpdate::default() {
        return Err(invalid(
            "Indicá al menos una sección: objective, decisions, openQuestions, topics o tasks. La sección enviada reemplaza a la guardada.",
        ));
    }
    Ok(update)
}

/// `add_meeting_notes`: what gets appended, without touching the rest.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NotesAddition {
    pub decisions: Vec<String>,
    pub open_questions: Vec<String>,
    /// Notes for a topic by its title (created at the end when missing).
    pub topic_items: Vec<(String, u64, Vec<String>)>,
    pub tasks: Vec<MeetingNoteTask>,
}

pub fn parse_notes_addition(arguments: &Value) -> Result<NotesAddition, BackendError> {
    let topic_items = arguments
        .get("topicItems")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let title = text(entry, "topic", MAX_TOPIC_TITLE_CHARS);
                    let at_ms = entry.get("minute").and_then(Value::as_str).and_then(parse_clock).unwrap_or(0);
                    let notes = items(entry, "items", crate::meeting_ai::MAX_TOPIC_ITEMS).unwrap_or_default();
                    (!title.is_empty() && !notes.is_empty()).then_some((title, at_ms, notes))
                })
                .take(MAX_TOPICS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let addition = NotesAddition {
        decisions: items(arguments, "decisions", MAX_LIST_ITEMS).unwrap_or_default(),
        open_questions: items(arguments, "openQuestions", MAX_LIST_ITEMS).unwrap_or_default(),
        topic_items,
        tasks: arguments
            .get("tasks")
            .and_then(Value::as_array)
            .map(|tasks| tasks.iter().filter_map(parsed_task).take(MAX_NOTE_TASKS).collect())
            .unwrap_or_default(),
    };
    if addition == NotesAddition::default() {
        return Err(invalid("Indicá qué agregar: decisions, openQuestions, topicItems o tasks."));
    }
    Ok(addition)
}

/// `set_meeting_ai_context`: the person's choice, as the picker stores it.
/// With `biblioteca` and no contexts, the caller allows the unlocked ones.
pub fn parse_context_choice(arguments: &Value) -> Result<MeetingContextChoice, BackendError> {
    let mode = arguments.get("mode").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    match mode.as_str() {
        "ninguno" => Ok(MeetingContextChoice { whole_library: false, folder: None, contexts: Vec::new() }),
        "carpeta" => {
            let folder = arguments.get("folder").and_then(Value::as_str).unwrap_or("");
            let folder = normalized_folder(folder)?
                .ok_or_else(|| invalid("Indicá la carpeta de la biblioteca en folder (ruta lógica, por ejemplo «Trabajo/Remesas»)."))?;
            Ok(MeetingContextChoice { whole_library: false, folder: Some(folder), contexts: Vec::new() })
        }
        "biblioteca" => {
            let mut seen = std::collections::HashSet::new();
            let contexts = arguments
                .get("contexts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|tag| {
                    if tag.trim().eq_ignore_ascii_case(UNTAGGED) {
                        Some(UNTAGGED.to_string())
                    } else {
                        normalize_context_tag(tag)
                    }
                })
                .filter(|tag| seen.insert(tag.to_lowercase()))
                .collect();
            Ok(MeetingContextChoice { whole_library: true, folder: None, contexts })
        }
        _ => Err(invalid(
            "Indicá mode: biblioteca (toda la biblioteca; contexts opcional la limita), carpeta (con folder) o ninguno.",
        )),
    }
}

/// `correct_meeting_transcript`: the (from, to) pairs to replace.
pub fn parse_replacements(arguments: &Value) -> Result<Vec<(String, String)>, BackendError> {
    let entries = arguments
        .get("replacements")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| invalid("Indicá replacements: [{from: \"palabra mal transcripta\", to: \"palabra correcta\"}]."))?;
    if entries.len() > MAX_TRANSCRIPT_REPLACEMENTS {
        return Err(invalid(format!("Hasta {MAX_TRANSCRIPT_REPLACEMENTS} reemplazos por llamada.")));
    }
    entries
        .iter()
        .map(|entry| {
            let from = entry.get("from").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let to = entry.get("to").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if from.chars().count() < 2 || from.chars().count() > MAX_REPLACEMENT_CHARS {
                return Err(invalid("Cada from necesita entre 2 y 120 caracteres."));
            }
            if to.is_empty() || to.chars().count() > MAX_REPLACEMENT_CHARS {
                return Err(invalid("Cada to necesita entre 1 y 120 caracteres."));
            }
            Ok((from, to))
        })
        .collect()
}

/// What the confirmation of a Meeting change shows, after checking its
/// input as the execution will. A transcript correction lists each pair
/// with how many times it changes the open meeting, tried in order on a
/// copy as the execution applies them; the other changes show what they
/// ask for. Without an open meeting only the AI context can change.
pub fn preview_text(name: &str, arguments: &Value, record: Option<&MeetingRecord>) -> Result<String, BackendError> {
    let open = || record.ok_or_else(|| invalid("No hay una reunión abierta en Meeting."));
    let (summary, detail) = match name {
        "correct_meeting_transcript" => {
            let replacements = parse_replacements(arguments)?;
            let mut copy = open()?.clone();
            let mut total = 0;
            let mut lines = Vec::new();
            for (from, to) in &replacements {
                let count = copy.replace_in_transcript(from, to);
                total += count;
                lines.push(format!("- «{from}» → «{to}»: {}", appearances(count)));
            }
            (format!("Corregir la transcripción: {} en total", appearances(total)), lines.join("\n"))
        }
        "update_meeting_notes" => {
            parse_notes_update(arguments)?;
            open()?;
            ("Reemplazar secciones de las Notas IA de la reunión".to_string(), pretty(arguments))
        }
        "add_meeting_notes" => {
            parse_notes_addition(arguments)?;
            open()?;
            ("Sumar a las Notas IA de la reunión".to_string(), pretty(arguments))
        }
        "set_meeting_ai_context" => {
            parse_context_choice(arguments)?;
            ("Cambiar el contexto que lee la IA de Meeting".to_string(), pretty(arguments))
        }
        _ => return Err(invalid("La herramienta de reunión no existe.")),
    };
    Ok(format!("{summary}\n{detail}"))
}

fn appearances(count: usize) -> String {
    match count {
        0 => "ninguna aparición".to_string(),
        1 => "1 aparición".to_string(),
        count => format!("{count} apariciones"),
    }
}

fn pretty(arguments: &Value) -> String {
    serde_json::to_string_pretty(arguments).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_update_takes_only_the_sections_it_names() {
        let update = parse_notes_update(&json!({
            "objective": " Cerrar remesas ",
            "tasks": ["Armar informe", { "text": "Revisar contratos", "owner": "Luis", "due": "martes" }],
        }))
        .expect("update");
        assert_eq!(update.objective.as_deref(), Some("Cerrar remesas"));
        assert_eq!(update.decisions, None);
        let tasks = update.tasks.expect("tasks");
        assert_eq!(tasks[0].text, "Armar informe");
        assert_eq!((tasks[1].owner.as_str(), tasks[1].due.as_str()), ("Luis", "martes"));
        assert!(parse_notes_update(&json!({})).is_err());
    }

    #[test]
    fn an_addition_needs_something_to_add() {
        let addition = parse_notes_addition(&json!({
            "topicItems": [{ "topic": "Remesas", "minute": "12:30", "items": ["El sandbox llega el viernes"] }],
        }))
        .expect("addition");
        assert_eq!(addition.topic_items, vec![("Remesas".to_string(), 750_000, vec!["El sandbox llega el viernes".to_string()])]);
        assert!(parse_notes_addition(&json!({ "topicItems": [{ "topic": "Sin notas" }] })).is_err());
    }

    #[test]
    fn the_context_choice_is_validated_by_mode() {
        let folder = parse_context_choice(&json!({ "mode": "carpeta", "folder": "/Trabajo/Remesas/" })).expect("folder");
        assert_eq!(folder.folder.as_deref(), Some("Trabajo/Remesas"));
        assert!(!folder.whole_library);
        let library = parse_context_choice(&json!({ "mode": "biblioteca", "contexts": ["laboral", "#Laboral", "sin-contexto"] })).expect("library");
        assert_eq!(library.contexts, vec!["#laboral".to_string(), UNTAGGED.to_string()]);
        assert!(parse_context_choice(&json!({ "mode": "carpeta" })).is_err());
        assert!(parse_context_choice(&json!({})).is_err());
    }

    fn record() -> crate::meeting::MeetingRecord {
        use crate::meeting::{MeetingRecord, MeetingSegment, MeetingSources, MeetingStart};
        let start = MeetingStart { date_label: "16/10/2026 10:00".into(), file_stamp: "2026-10-16 10.00".into(), unix_ms: 0 };
        let mut record = MeetingRecord::new("m1", start, MeetingSources { microphone: true, system: false }, false);
        record.complete(
            vec![
                MeetingSegment { start_ms: 0, end_ms: 1_000, speaker_id: Some("speaker-1".into()), text: "El carreo llegó tarde, revisen el carreo.".into() },
                MeetingSegment { start_ms: 1_000, end_ms: 2_000, speaker_id: Some("speaker-2".into()), text: "Carreo corporativo, no carreos personales.".into() },
            ],
            2_000,
        );
        record
    }

    #[test]
    fn a_misheard_word_is_replaced_whole_and_keeps_its_capital() {
        let mut record = record();
        assert_eq!(record.replace_in_transcript("carreo", "correo"), 3);
        let text = record.snapshot(&crate::meeting::MeetingFilter::default()).turns.iter().map(|turn| turn.text.clone()).collect::<Vec<_>>().join(" | ");
        assert_eq!(text, "El correo llegó tarde, revisen el correo. | Correo corporativo, no carreos personales.");
        assert_eq!(record.replace_in_transcript("inexistente", "algo"), 0);
    }

    #[test]
    fn an_update_replaces_sections_and_keeps_sent_tasks() {
        let mut record = record();
        record.apply_notes_update(parse_notes_update(&json!({
            "objective": "Cerrar remesas",
            "decisions": ["Corte a las 18"],
            "tasks": [{ "text": "Armar informe", "owner": "Luis" }],
        })).expect("first"));
        let first_id = record.ai_notes.notes.tasks[0].id.clone();
        assert!(!first_id.is_empty());
        record.ai_notes.notes.tasks[0].sent = true;
        record.apply_notes_update(parse_notes_update(&json!({
            "tasks": [{ "text": "armar  informe", "owner": "Luis" }, { "text": "Revisar contratos" }],
        })).expect("second"));
        let notes = &record.ai_notes.notes;
        // The untouched sections stay; the matched task keeps id and sent.
        assert_eq!(notes.objective, "Cerrar remesas");
        assert_eq!(notes.decisions, vec!["Corte a las 18".to_string()]);
        assert_eq!((notes.tasks[0].id.as_str(), notes.tasks[0].sent), (first_id.as_str(), true));
        assert!(!notes.tasks[1].id.is_empty());
        assert_ne!(notes.tasks[1].id, notes.tasks[0].id);
    }

    #[test]
    fn an_addition_appends_without_repeating() {
        let mut record = record();
        record.apply_notes_update(parse_notes_update(&json!({ "decisions": ["Corte a las 18"] })).expect("base"));
        let added = record.apply_notes_addition(parse_notes_addition(&json!({
            "decisions": ["corte a las 18", "Eva valida los montos"],
            "topicItems": [{ "topic": "Remesas", "minute": "01:00", "items": ["Sandbox el viernes"] }],
            "tasks": ["Revisar contratos"],
        })).expect("addition"));
        let notes = &record.ai_notes.notes;
        assert_eq!(added, 3);
        assert_eq!(notes.decisions, vec!["Corte a las 18".to_string(), "Eva valida los montos".to_string()]);
        assert_eq!((notes.topics[0].title.as_str(), notes.topics[0].at_ms, notes.topics[0].items.as_slice()), ("Remesas", 60_000, &["Sandbox el viernes".to_string()][..]));
        assert!(!notes.tasks[0].id.is_empty());
    }

    #[test]
    fn replacements_are_bounded_pairs() {
        let replacements = parse_replacements(&json!({ "replacements": [{ "from": "carreo", "to": "correo" }] })).expect("pairs");
        assert_eq!(replacements, vec![("carreo".to_string(), "correo".to_string())]);
        assert!(parse_replacements(&json!({ "replacements": [{ "from": "a", "to": "b" }] })).is_err());
        assert!(parse_replacements(&json!({ "replacements": [] })).is_err());
    }

    #[test]
    fn the_confirmation_counts_each_replacement_without_changing_the_meeting() {
        let record = record();
        let arguments = json!({ "replacements": [
            { "from": "carreo", "to": "correo" },
            { "from": "correo corporativo", "to": "correo de la empresa" },
            { "from": "inexistente", "to": "algo" },
        ] });
        let text = preview_text("correct_meeting_transcript", &arguments, Some(&record)).expect("preview");
        assert_eq!(
            text,
            "Corregir la transcripción: 4 apariciones en total\n\
             - «carreo» → «correo»: 3 apariciones\n\
             - «correo corporativo» → «correo de la empresa»: 1 aparición\n\
             - «inexistente» → «algo»: ninguna aparición"
        );
        assert_eq!(record, self::record(), "the preview works on a copy");
        assert!(preview_text("correct_meeting_transcript", &arguments, None).is_err());
        assert!(preview_text("add_meeting_notes", &json!({}), Some(&record)).is_err());
        assert!(preview_text("set_meeting_ai_context", &json!({ "mode": "ninguno" }), None).is_ok());
    }
}
