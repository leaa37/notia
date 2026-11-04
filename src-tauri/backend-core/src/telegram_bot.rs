//! Conversation rules of the Telegram bot that are independent of the
//! transport: replies to confirmations and choices, the texts the bot sends
//! (confirmation, plan, progress, queue) and which requests are financial.
//! The worker in the Tauri adapter polls Telegram and runs the agent.

use std::sync::OnceLock;

use regex::Regex;

use crate::events::BackendEvent;
use crate::protocol::{BackendMessage, MessageRole};
use crate::chat_attachments::MessageAttachment;
use crate::formatting::{escape_telegram_html, markdown_to_telegram_html};
use crate::interaction::ExecutionPlan;
use crate::protocol::MutationPreview;

pub const MAX_PENDING_REQUESTS: usize = 10;
pub const RECOVERY_COMMAND: &str = "/reanudar";
pub const MAX_HISTORY_MESSAGES: usize = 20;
/// Files of earlier requests a new request still shows: an album's worth.
pub const MAX_HISTORY_FILES: usize = 10;
/// Messages of a request: the chat's history and the new message with its
/// own files. A file sent earlier stays on the message it came with (the
/// latest `MAX_HISTORY_FILES` of the history), so the model still sees it
/// when the person asks about it later, after the answer it already got.
/// Moved to the new message, a lunch photo looked freshly sent for hours:
/// every later request, and Notia's own messages, offered to load it again.
pub fn turn_messages(history: &[BackendMessage], text: &str, attachments: Vec<MessageAttachment>) -> Vec<BackendMessage> {
    let total = history.iter().map(|message| message.attachments.len()).sum::<usize>();
    let mut dropped = total.saturating_sub(MAX_HISTORY_FILES);
    let mut messages = history
        .iter()
        .map(|message| {
            let skip = dropped.min(message.attachments.len());
            dropped -= skip;
            BackendMessage { attachments: message.attachments[skip..].to_vec(), ..message.clone() }
        })
        .collect::<Vec<_>>();
    messages.push(BackendMessage {
        role: MessageRole::User,
        content: text.to_string(),
        images: Vec::new(),
        attachments,
    });
    messages
}

fn fold(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// `true`/`false` for a typed yes/no answer to a pending confirmation.
pub fn parse_confirmation_decision(value: &str) -> Option<ConfirmationReply> {
    let normalized = fold(value)
        .chars()
        .map(|character| if ",.!?¿¡".contains(character) { ' ' } else { character })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match normalized.as_str() {
        "si" | "confirmo" | "si confirmo" | "confirmar" | "acepto" => Some(ConfirmationReply::Confirm),
        "confirmar todos" | "confirmar todo" | "confirmo todos" | "confirmo todo" | "si a todo" | "si a todos" | "todos"
        | "aprobar todo" | "apruebo todo" => Some(ConfirmationReply::ConfirmAll),
        "proponer otra cosa" | "propongo otra cosa" | "otra cosa" => Some(ConfirmationReply::Propose),
        "no" | "cancelo" | "no confirmo" | "cancelar" | "rechazo" => Some(ConfirmationReply::Cancel),
        _ => None,
    }
}

/// What the person chose for a change or a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationReply {
    Confirm,
    /// This one and every change or plan the rest of the request asks for.
    ConfirmAll,
    /// Not this: the person writes what to do instead.
    Propose,
    Cancel,
}

impl ConfirmationReply {
    /// Id carried by the button's callback data.
    pub fn id(self) -> &'static str {
        match self {
            Self::Confirm => "yes",
            Self::ConfirmAll => "all",
            Self::Propose => "other",
            Self::Cancel => "no",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Confirm, Self::ConfirmAll, Self::Propose, Self::Cancel].into_iter().find(|reply| reply.id() == id)
    }
}

/// Buttons of a change or a plan, in order, as (label, reply).
pub const CONFIRMATION_BUTTONS: [(&str, ConfirmationReply); 4] = [
    ("Confirmar", ConfirmationReply::Confirm),
    ("Confirmar todos", ConfirmationReply::ConfirmAll),
    ("Proponer otra cosa", ConfirmationReply::Propose),
    ("Cancelar", ConfirmationReply::Cancel),
];

const CONFIRMATION_CHOICES: &str =
    "Elegí Confirmar, Confirmar todos (no te vuelvo a preguntar en este pedido), Proponer otra cosa o Cancelar.";

/// What the person chose on a confirmation or a plan (`None`: nobody
/// answered in time), as the line that replaces its choices.
pub fn confirmation_outcome(reply: Option<ConfirmationReply>) -> &'static str {
    match reply {
        Some(ConfirmationReply::Confirm) => "Elegiste: Confirmar.",
        Some(ConfirmationReply::ConfirmAll) => "Elegiste: Confirmar todos. No te vuelvo a preguntar en este pedido.",
        Some(ConfirmationReply::Propose) => "Elegiste: Proponer otra cosa.",
        Some(ConfirmationReply::Cancel) => "Elegiste: Cancelar. No se aplicaron cambios.",
        None => "Venció sin respuesta. No se aplicaron cambios.",
    }
}

/// Longest answer an answered question repeats.
const MAX_ANSWER_ECHO_CHARS: usize = 200;

/// The answer to a question of the agent (`None`: nobody answered in
/// time), as the line under it.
pub fn question_outcome(answer: Option<&str>) -> String {
    match answer.map(str::trim) {
        Some(answer) if answer.chars().count() > MAX_ANSWER_ECHO_CHARS => {
            format!("Respuesta: {}…", answer.chars().take(MAX_ANSWER_ECHO_CHARS).collect::<String>())
        }
        Some(answer) => format!("Respuesta: {answer}"),
        None => "La pregunta venció sin respuesta.".to_string(),
    }
}

/// The last message of an answered confirmation, plan or question (HTML):
/// its choices line gives way to `outcome`. It is sent without buttons.
pub fn answered_prompt(html: &str, outcome: &str) -> String {
    let body = html.trim_end();
    let body = body.strip_suffix(CONFIRMATION_CHOICES).unwrap_or(body).trim_end();
    format!("{body}\n\n<b>{}</b>", escape_telegram_html(outcome))
}

/// A typed 1-based number selects the same option shown as a button.
pub fn resolve_choice_reply(value: &str, choices: &[String]) -> (String, Option<usize>) {
    let trimmed = value.trim();
    let number = trimmed.trim_end_matches(['.', ')']).trim();
    match number.parse::<usize>().ok().and_then(|index| index.checked_sub(1)) {
        Some(index) if index < choices.len() => (choices[index].clone(), Some(index)),
        _ => (trimmed.to_string(), None),
    }
}

/// Removes operation ids, secrets and private paths from text shown in a chat.
pub fn redact_detail(value: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"(?i)\boperationId\s*[:=]\s*[^\s,;]+", "operationId=[oculto]"),
            (r"(?i)((?:api[_ -]?key|access[_ -]?token|password|passwd|secret|cookie))\s*[:=]\s*[^\s,;}]+", "$1=[oculto]"),
            (r#"(?i)(?:[A-Za-z]:[\\/]|/(?:Users|home|private|appdata|documents)[\\/])[^\s"']+"#, "[ruta privada]"),
        ]
        .into_iter()
        .map(|(pattern, replacement)| (Regex::new(pattern).expect("static redaction pattern"), replacement))
        .collect()
    });
    let mut text = value.to_string();
    for (pattern, replacement) in rules {
        text = pattern.replace_all(&text, *replacement).into_owned();
    }
    text.trim().to_string()
}

/// Longest agent note the progress message shows.
const MAX_NOTE_CHARS: usize = 300;

/// Confirmation text for a prepared mutation, as the HTML of one or more
/// messages; the buttons go with the last one.
pub fn confirmation_parts(preview: &MutationPreview) -> Vec<String> {
    // A change outside the documents (a routine, a ticket) has no count to give.
    let counts = (!preview.documents.is_empty())
        .then(|| format!("Documentos afectados: {}. Cambios preparados: {}.", preview.documents.len(), preview.hunks.len()))
        .unwrap_or_default();
    let detail = [redact_detail(&preview.summary), counts]
    .into_iter()
    .filter(|line| !line.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    plain_parts(&format!(
        "Confirmación requerida:\n\n{detail}\n\n{CONFIRMATION_CHOICES}"
    ))
}

/// Approval text for an execution plan, as the HTML of one or more
/// messages; the buttons go with the last one.
pub fn plan_parts(plan: &ExecutionPlan) -> Vec<String> {
    let steps = plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| format!("{}. {}", index + 1, redact_detail(&step.label)))
        .collect::<Vec<_>>()
        .join("\n");
    plain_parts(&format!(
        "Aprobar este plan de ejecución:\n{steps}\n\n{CONFIRMATION_CHOICES}"
    ))
}

/// Longest text of an option button; Telegram shows short labels.
pub const MAX_BUTTON_CHARS: usize = 48;

/// A question of the agent (Markdown) as the HTML of one or more messages;
/// the option buttons go with the last one. Options too long for a button
/// are also listed in the text, so none is cut.
pub fn question_parts(question: &str, options: &[String]) -> Vec<String> {
    let mut markdown = question.trim().to_string();
    if options.iter().any(|option| option.chars().count() > MAX_BUTTON_CHARS) {
        let listed = options.iter().enumerate().map(|(index, option)| format!("{}. {option}", index + 1)).collect::<Vec<_>>();
        markdown.push_str(&format!("\n\n{}", listed.join("\n")));
    }
    telegram_html_parts(&markdown)
}

/// Plain text as the HTML of one or more messages, split between lines.
fn plain_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::<String>::new();
    let mut current = String::new();
    for line in text.lines().flat_map(|line| split_long_line(line, MAX_MESSAGE_HTML / 2)) {
        let joined = if current.is_empty() { line.clone() } else { format!("{current}\n{line}") };
        if escape_telegram_html(&joined).chars().count() <= MAX_MESSAGE_HTML {
            current = joined;
        } else {
            parts.push(escape_telegram_html(&current));
            current = line;
        }
    }
    if !current.trim().is_empty() {
        parts.push(escape_telegram_html(&current));
    }
    parts
}

/// A line cut at spaces (or anywhere, for a word longer than the limit)
/// into pieces of at most `max_chars`.
fn split_long_line(line: &str, max_chars: usize) -> Vec<String> {
    if line.chars().count() <= max_chars {
        return vec![line.to_string()];
    }
    let mut pieces = Vec::<String>::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if candidate.chars().count() <= max_chars {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
        }
        let characters = word.chars().collect::<Vec<_>>();
        let mut chunks = characters.chunks(max_chars).map(|chunk| chunk.iter().collect::<String>()).collect::<Vec<_>>();
        current = chunks.pop().unwrap_or_default();
        pieces.extend(chunks);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Whether a message is about mail or the calendar. Those requests stay in
/// the library, where the Gmail and Calendar tools are, even when they say
/// «cuenta» («de mi cuenta de gmail»).
pub fn is_mail_request(value: &str) -> bool {
    static TERMS: OnceLock<Regex> = OnceLock::new();
    TERMS
        .get_or_init(|| {
            Regex::new(r"\b(gmail|correos?|e-?mails?|mails?|bandeja de entrada|casilla de correo|calendario|google calendar)\b").expect("mail terms")
        })
        .is_match(&fold(value))
}

/// Whether a message is about personal finances; documents always are.
/// Mail and calendar requests are not, even when they name an account.
pub fn is_finance_request(value: &str) -> bool {
    if is_mail_request(value) {
        return false;
    }
    static TERMS: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
    let (terms, amount, verb) = TERMS.get_or_init(|| {
        (
            Regex::new(r"\b(finanzas?|financier[oa]s?|gast(?:o|os|e|aste|amos|ar)|pague|pagaste|pago|cobre|cobraste|cobro|ingreso|ingresos|saldo|saldos|cuenta|cuentas|categoria|categorias|ahorro|ahorros|retiro|aporte|transferencia|movimiento|movimientos|sueldo|ticket|factura|boleta|servicio|luz|gas|internet|precio|nafta|combustible|cotizacion(?:es)?|dolar(?:es)?|inflacion|ipc|oficial|blue)\b").expect("finance terms"),
            Regex::new(r"(?:\$\s*\d|\b\d+(?:[.,]\d{1,2})?\s*(?:ars|usd|pesos?)\b)").expect("money amount"),
            Regex::new(r"\b(carg(?:a|ue|aste|amos|ar|ado)|anot(?:a|alo|arla|ar|e|aste|amos|ado)|registr(?:a|alo|arla|ar|e|aste|amos|ado))\b").expect("finance verb"),
        )
    });
    let normalized = fold(value);
    terms.is_match(&normalized) || (amount.is_match(&normalized) && verb.is_match(&normalized))
}

/// What the agent is doing, in words for the user.
pub fn tool_label(tool: &str) -> &'static str {
    match tool {
        "search_library_documents" | "search_library_files" => "buscando en la biblioteca",
        "search_library_context" => "consultando el contexto de la biblioteca",
        "read_library_document" | "read_library_documents" => "leyendo documentos autorizados",
        "move_library_entry" => "ordenando la biblioteca",
        "read_all_task_tickets" | "search_task_tickets" | "get_task_board_summary" => "consultando las tareas",
        "search_web" => "consultando fuentes públicas",
        "get_weather" => "consultando el clima",
        "add_agent_thought" => "anotando un pensamiento",
        "add_agent_biography" => "sumando a tu biografía",
        "add_agent_talk" => "anotando cómo hablás",
        "add_agent_person" => "anotando quién es quién",
        "add_agent_improvement" => "anotando una mejora para Notia",
        "create_ai_task" => "preparando la tarea",
        "list_ai_tasks" | "get_ai_task" => "revisando tus tareas IA",
        "cancel_ai_task" | "resume_ai_task" | "review_ai_task" | "delete_ai_task" => "preparando el cambio en la tarea",
        "wait_ai_task" => "dejando la tarea esperando",
        crate::terminal::TERMINAL_TOOL => "ejecutando un comando en la terminal",
        crate::browser::NAVIGATE_TOOL => "abriendo la página en el navegador",
        crate::browser::SNAPSHOT_TOOL => "leyendo la página del navegador",
        crate::browser::SCREENSHOT_TOOL => "mirando la página del navegador",
        crate::browser::CLICK_TOOL => "haciendo clic en el navegador",
        crate::browser::TYPE_TOOL => "escribiendo en el navegador",
        name if crate::browser::is_browser_tool(name) => "usando el navegador",
        crate::agent_scripts::LIST_TOOL => "revisando los scripts de soporte",
        crate::agent_scripts::RUN_TOOL => "corriendo un script de soporte",
        crate::agent_scripts::SAVE_TOOL => "escribiendo un script de soporte",
        crate::whatsapp::LIST_CHATS_TOOL => "buscando el chat de WhatsApp",
        crate::whatsapp::READ_MESSAGES_TOOL => "leyendo el chat de WhatsApp",
        crate::whatsapp::SEND_MESSAGE_TOOL => "escribiendo por WhatsApp",
        crate::whatsapp::SEND_FILE_TOOL => "mandando el archivo por WhatsApp",
        crate::whatsapp::GET_IMAGE_TOOL => "mirando la imagen de WhatsApp",
        "generate_image" => "generando la imagen",
        "edit_image" => "editando la imagen",
        "list_gmail_messages" | "read_gmail_message" | "list_gmail_labels" => "revisando tu correo",
        "send_gmail_message" => "preparando el correo",
        "trash_gmail_messages" | "move_gmail_messages" | "mark_gmail_spam" | "mark_gmail_read" => "preparando el cambio en tu correo",
        "list_calendar_events" => "revisando tu calendario",
        "create_calendar_event" => "preparando el evento del calendario",
        "request_user_clarification" => "preparando una pregunta para vos",
        "get_finance_dollar_quotes" | "get_finance_historical_dollar_quotes" => "consultando cotizaciones",
        "get_finance_inflation_indices" => "consultando índices económicos",
        "extract_finance_document" => "leyendo el documento financiero",
        name if name.starts_with("get_finance_") || name.starts_with("list_finance_") => "consultando tus finanzas",
        name if name.starts_with("create_finance_") || name.starts_with("save_finance_") => "preparando el registro financiero",
        "get_gym_summary" | "get_gym_routine" | "list_gym_workouts" => "revisando tus entrenamientos",
        "search_gym_exercises" | "get_gym_exercise" | "list_gym_equipment" => "buscando ejercicios",
        "control_gym_session" => "actualizando tu entrenamiento",
        name if crate::gym::tools::is_gym_write_tool(name) => "preparando el cambio en Gimnasio",
        name if name.starts_with("get_routine_") || name.starts_with("list_routine_") => "consultando tu rutina",
        "set_routine_completions" => "registrando tus hábitos",
        name if name.contains("routine") => "preparando el cambio en tu rutina",
        "list_recipes" | "get_recipe" => "revisando tus recetas",
        name if name.contains("recipe") => "preparando la receta",
        crate::tool_routing::SWITCH_AREA_TOOL => "buscando las herramientas que hacen falta",
        "get_health_summary" | "list_health_records" => "revisando tus datos de salud",
        "log_meal" | "update_meal" => "registrando tu comida",
        "log_weight" => "registrando tu peso",
        "log_water" => "registrando el agua",
        "log_steps" => "registrando tus pasos",
        "log_sleep" => "registrando tu sueño",
        "set_health_plan" => "preparando tu plan",
        name if crate::health::tools::is_health_write_tool(name) => "preparando el cambio en Salud",
        "list_ai_actions" | "get_ai_action" => "revisando tus acciones programadas",
        name if name.contains("ai_action") => "preparando el cambio en tus acciones programadas",
        "list_agenda" => "leyendo tu agenda",
        name if name.contains("agenda") => "preparando el cambio en tu agenda",
        name if name.contains("task") => "preparando el cambio en tareas",
        name if name.contains("library") || name.contains("markdown") || name.contains("document") => "preparando el cambio en la biblioteca",
        _ => "completando una tarea autorizada",
    }
}

fn phase_label(phase: &str) -> &'static str {
    match phase {
        "planning" => "Organizando los pasos",
        "reading" => "Leyendo la información necesaria",
        "searching" => "Buscando información pública",
        "responding" => "Redactando la respuesta",
        "executing" => "Ejecutando la operación autorizada",
        "waiting-clarification" => "Necesito una aclaración",
        "waiting-confirmation" => "Espero tu confirmación",
        "verifying" => "Verificando el resultado",
        _ => "Preparando la solicitud",
    }
}

/// Progress text after the events seen so far (HTML), or `None` before the
/// agent starts.
pub fn progress_message(events: &[BackendEvent]) -> Option<String> {
    let mut phase = None;
    let mut activity = None;
    let mut round = None;
    let mut note = None;
    for event in events {
        match event {
            BackendEvent::AssistantNote { text, .. } => note = Some(text.as_str()),
            BackendEvent::PhaseChanged { phase: value, round: value_round, .. } => {
                phase = Some(phase_label(value));
                if value_round.is_some() {
                    round = *value_round;
                }
            }
            BackendEvent::RoundStarted { round: value, .. } => round = Some(*value),
            BackendEvent::ToolStarted { tool_name, .. } => activity = Some(tool_label(tool_name)),
            BackendEvent::ToolCompleted { .. } => activity = None,
            BackendEvent::ClarificationRequired { .. } => phase = Some(phase_label("waiting-clarification")),
            BackendEvent::ConfirmationRequired { .. } => phase = Some(phase_label("waiting-confirmation")),
            _ => {}
        }
    }
    let phase = phase.or(round.map(|_| phase_label("reading")))?;
    let step = round.map(|value| format!(" (paso {value})")).unwrap_or_default();
    // The agent's last note is its status, as a command-line agent shows it,
    // in plain text: a clipped Markdown would show its marks.
    let note = note
        .map(|text| format!("\n<i>{}</i>", escape_telegram_html(&clipped_note(&plain_text(text)))))
        .unwrap_or_default();
    Some(match activity {
        Some(activity) => format!("<b>{phase}</b>{step}{note}\nAhora: {activity}."),
        None => format!("<b>{phase}</b>{step}{note}"),
    })
}

/// The progress keeps every event but the streamed text; the output of a
/// command joins the previous one, so a long command stays one event.
pub fn keep_progress_event(seen: &mut Vec<BackendEvent>, event: BackendEvent) {
    if matches!(event, BackendEvent::ThinkingSummary { .. } | BackendEvent::AssistantDelta { .. }) {
        return;
    }
    if let Some(last) = seen.last_mut() {
        if crate::terminal::join_output(last, &event) {
            return;
        }
    }
    seen.push(event);
}

/// The worker posts a command's message again (below, so it notifies)
/// this often while the command runs; in between it is edited.
pub const TERMINAL_REPOST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Lines and characters of output a command's message shows.
const TERMINAL_TAIL_LINES: usize = 15;
const TERMINAL_TAIL_CHARS: usize = 1_500;

/// The message (HTML) that follows a command of the terminal: what runs,
/// for how long and the last lines it printed; how it ended once it did.
pub fn terminal_message(run: &crate::terminal::TerminalRun, elapsed: std::time::Duration) -> String {
    let command = run.command.trim();
    let command = if command.chars().count() > 300 { format!("{}…", command.chars().take(300).collect::<String>()) } else { command.to_string() };
    let title = match (&run.ended, run.duration) {
        (Some(summary), Some(duration)) => format!("Terminal: {summary} en {}", crate::terminal::duration_text(duration)),
        (Some(summary), None) => format!("Terminal: {summary}"),
        (None, _) => format!("Terminal: ejecutando hace {}", crate::terminal::duration_text(elapsed)),
    };
    // What the light agent read in the live output, while it runs.
    let status = match (&run.status, &run.ended) {
        (Some(status), None) => format!("\n<i>{}</i>", escape_telegram_html(status)),
        _ => String::new(),
    };
    let tail = run.tail(TERMINAL_TAIL_LINES, TERMINAL_TAIL_CHARS);
    let output = if tail.trim().is_empty() {
        if run.ended.is_some() { "<i>Sin salida.</i>".to_string() } else { "<i>Todavía sin salida.</i>".to_string() }
    } else {
        format!("<pre>{}</pre>", escape_telegram_html(tail.trim_end()))
    };
    format!("<b>{}</b>{status}\n<code>{}</code>\n{output}", escape_telegram_html(&title), escape_telegram_html(&command))
}

/// Markdown as one line of plain text: without headings, emphasis, code
/// marks or table bars.
fn plain_text(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim_start_matches(['-', '*', '•', '>']).trim())
        .filter(|line| !line.is_empty() && !line.chars().all(|character| matches!(character, '|' | '-' | ':' | ' ')))
        .map(|line| line.replace("**", "").replace("__", "").replace('`', "").replace('|', " "))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Notes of the agent the person has not read whole, in order: the ones the
/// progress showed clipped and, before a question (`asking`), the note of
/// the round that asks, which is its context. A short status note was read
/// in the progress while it lasted.
pub fn notes_to_deliver(events: &[BackendEvent], asking: bool) -> Vec<String> {
    let mut notes = Vec::<(String, bool)>::new();
    for event in events {
        match event {
            BackendEvent::RoundStarted { .. } => notes.iter_mut().for_each(|(_, current)| *current = false),
            BackendEvent::AssistantNote { text, .. } if !text.trim().is_empty() => notes.push((text.trim().to_string(), true)),
            _ => {}
        }
    }
    notes
        .into_iter()
        .filter(|(text, current)| text.chars().count() > MAX_NOTE_CHARS || (asking && *current))
        .map(|(text, _)| text)
        .collect()
}

/// Longest HTML of one message; Telegram accepts 4096 characters.
const MAX_MESSAGE_HTML: usize = 3_800;

/// A Markdown answer as the HTML of one or more Telegram messages, split
/// between paragraphs (a long paragraph between lines) so no tag is cut and
/// nothing is lost. A code block is never split.
pub fn telegram_html_parts(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::<String>::new();
    let mut in_code = false;
    for line in markdown.trim().lines() {
        let fence = line.trim_start().starts_with("```");
        match blocks.last_mut() {
            Some(block) if in_code || !line.trim().is_empty() && !block.is_empty() => {
                block.push('\n');
                block.push_str(line);
            }
            _ if line.trim().is_empty() => blocks.push(String::new()),
            _ => blocks.push(line.to_string()),
        }
        if fence {
            in_code = !in_code;
        }
    }
    let mut parts = Vec::<String>::new();
    let mut current = String::new();
    for block in blocks.into_iter().filter(|block| !block.trim().is_empty()) {
        for piece in fitting_pieces(&block) {
            let joined = if current.is_empty() { piece.clone() } else { format!("{current}\n\n{piece}") };
            if markdown_to_telegram_html(&joined).chars().count() <= MAX_MESSAGE_HTML {
                current = joined;
            } else {
                if !current.is_empty() {
                    parts.push(markdown_to_telegram_html(&current));
                }
                current = piece;
            }
        }
    }
    if !current.is_empty() {
        parts.push(markdown_to_telegram_html(&current));
    }
    parts
}

/// A block that fits in a message, or its lines grouped so they do. A code
/// block keeps its fence in every piece; a line too long is cut at spaces.
fn fitting_pieces(block: &str) -> Vec<String> {
    if markdown_to_telegram_html(block).chars().count() <= MAX_MESSAGE_HTML {
        return vec![block.to_string()];
    }
    let fits = |piece: &str| markdown_to_telegram_html(piece).chars().count() <= MAX_MESSAGE_HTML;
    if block.trim_start().starts_with("```") {
        let mut lines = block.lines().collect::<Vec<_>>();
        let opening = lines.remove(0).trim().to_string();
        if lines.last().is_some_and(|line| line.trim().starts_with("```")) {
            lines.pop();
        }
        let fence = |body: &str| format!("{opening}\n{body}\n```");
        let mut pieces = Vec::<String>::new();
        let mut body = String::new();
        for line in lines.iter().flat_map(|line| split_long_line(line, MAX_MESSAGE_HTML / 2)) {
            let joined = if body.is_empty() { line.clone() } else { format!("{body}\n{line}") };
            if fits(&fence(&joined)) {
                body = joined;
            } else {
                if !body.is_empty() {
                    pieces.push(fence(&body));
                }
                body = line;
            }
        }
        if !body.is_empty() {
            pieces.push(fence(&body));
        }
        return pieces;
    }
    let mut pieces = Vec::<String>::new();
    for line in block.lines().flat_map(|line| split_long_line(line, MAX_MESSAGE_HTML / 2)) {
        let line = line.as_str();
        match pieces.last_mut() {
            Some(piece) if markdown_to_telegram_html(&format!("{piece}\n{line}")).chars().count() <= MAX_MESSAGE_HTML => {
                piece.push('\n');
                piece.push_str(line);
            }
            _ => pieces.push(line.to_string()),
        }
    }
    pieces
}

fn clipped_note(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > MAX_NOTE_CHARS {
        format!("{}…", text.chars().take(MAX_NOTE_CHARS).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Reply when a message asks to stop the running request (HTML).
pub const CANCELLING_MESSAGE: &str = "<b>Cancelando la solicitud en curso…</b>";

/// Reply once the running request stopped.
pub const CANCELLED_MESSAGE: &str = crate::turn_interrupts::CANCELLED_REPLY;

/// Reply when the request finished before the cancellation reached it.
pub const TOO_LATE_TO_CANCEL_MESSAGE: &str = "La solicitud ya había terminado cuando llegó la cancelación.";

/// Reply when a request waits behind others (HTML).
pub fn queued_message(ahead: usize, document: bool) -> String {
    let what = if document { "Documento en cola." } else { "Solicitud en cola." };
    let plural = if ahead == 1 { "" } else { "es" };
    format!("<b>{what}</b> Hay {ahead} solicitud{plural} antes.")
}

/// Notice about requests interrupted by a restart.
pub fn interrupted_message(count: usize) -> String {
    if count == 1 {
        format!("Tengo una solicitud interrumpida con estado desconocido. Escribí {RECOVERY_COMMAND} si querés reanudarla; no la voy a repetir automáticamente.")
    } else {
        format!("Tengo {count} solicitudes interrumpidas con estado desconocido. Escribí {RECOVERY_COMMAND} si querés reanudarlas; no las voy a repetir automáticamente.")
    }
}

/// Request of a photo, image or PDF sent without text: the model decides
/// what it is, since the router and the agent both see the attachment.
pub const DOCUMENT_PROMPT: &str = "[Origen: documento de Telegram sin texto. Mirá su contenido y hacé lo que corresponda: si es un comprobante financiero (ticket de compra, factura o boleta de servicio, recibo de sueldo, resumen de tarjeta de crédito), extraé todos los campos legibles y registralo con la herramienta financiera del tipo detectado, sin duplicar como gasto una factura de servicio; si es la foto de un plato o una comida, registrala con log_meal y photoFromMessage reconstruyendo qué es (nombre, descripción e ingredientes con pesos estimados en gramos): Notia la busca o la crea en Recetas y la carga en Salud; si es otra cosa (por ejemplo, una agenda o un calendario, una tarea o apuntes), usá las herramientas que correspondan o preguntá qué hacer. Si son varios, procesalos todos.]";

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(name: &str) -> MessageAttachment {
        MessageAttachment {
            name: name.into(),
            media_type: "image/jpeg".into(),
            kind: crate::chat_attachments::MessageAttachmentKind::Image,
            pages: vec!["aW1n".into()],
            text_content: None,
            extracted_text: None,
            page_count: None,
        }
    }

    fn said(role: MessageRole, content: &str, attachments: Vec<MessageAttachment>) -> BackendMessage {
        BackendMessage { role, content: content.into(), images: Vec::new(), attachments }
    }

    #[test]
    fn a_command_shows_what_runs_its_last_lines_and_how_it_ended() {
        let started = BackendEvent::TerminalStarted {
            request_id: "r".into(),
            call_id: "c".into(),
            command: "npm run build <prod>".into(),
            shell: "powershell".into(),
            cwd: "C:\\repo".into(),
        };
        let output = |text: &str| BackendEvent::TerminalOutput { request_id: "r".into(), call_id: "c".into(), text: text.into(), partial: "60%".into() };
        let mut seen = Vec::new();
        keep_progress_event(&mut seen, started);
        keep_progress_event(&mut seen, BackendEvent::ThinkingSummary { request_id: "r".into(), summary: "pienso".into() });
        for index in 0..40 {
            keep_progress_event(&mut seen, output(&format!("paso {index}\n")));
        }
        // The output joined into one event after the start.
        assert_eq!(seen.len(), 2);
        let runs = crate::terminal::terminal_runs(&seen);
        let running = terminal_message(&runs[0], std::time::Duration::from_secs(185));
        assert!(running.starts_with("<b>Terminal: ejecutando hace 3 min 5 s</b>\n<code>npm run build &lt;prod&gt;</code>"));
        assert!(running.contains("paso 39\n60%</pre>"));
        keep_progress_event(&mut seen, BackendEvent::TerminalStatus { request_id: "r".into(), call_id: "c".into(), status: "Va por el paso 39 & sin errores.".into() });
        let runs = crate::terminal::terminal_runs(&seen);
        let with_status = terminal_message(&runs[0], std::time::Duration::from_secs(190));
        assert!(with_status.starts_with("<b>Terminal: ejecutando hace 3 min 10 s</b>\n<i>Va por el paso 39 &amp; sin errores.</i>\n<code>"));
        assert!(!running.contains("paso 20\n"));
        keep_progress_event(&mut seen, BackendEvent::TerminalFinished {
            request_id: "r".into(),
            call_id: "c".into(),
            status: "failed".into(),
            exit_code: Some(1),
            duration_ms: 200_000,
            summary: "terminó con código 1".into(),
            markdown: String::new(),
        });
        let runs = crate::terminal::terminal_runs(&seen);
        let ended = terminal_message(&runs[0], std::time::Duration::from_secs(999));
        assert!(ended.starts_with("<b>Terminal: terminó con código 1 en 3 min 20 s</b>"));
        assert!(ended.contains("paso 39</pre>"));
        assert!(!ended.contains("<i>"));
    }

    #[test]
    fn a_photo_sent_earlier_stays_on_its_message() {
        let history = vec![
            said(MessageRole::User, "Cargame este almuerzo", vec![photo("almuerzo.jpg")]),
            said(MessageRole::Assistant, "Quedó registrado.", Vec::new()),
        ];
        let messages = turn_messages(&history, "¿Me pasás la rutina de hoy?", Vec::new());
        assert_eq!(messages.len(), 3);
        // The model still sees the photo, where it was sent and answered.
        assert_eq!(messages[0].attachments, vec![photo("almuerzo.jpg")]);
        assert!(messages[1].attachments.is_empty());
        // The new request is not presented as carrying it.
        assert_eq!(messages[2].content, "¿Me pasás la rutina de hoy?");
        assert!(messages[2].attachments.is_empty());
        // Only the latest files of the history stay; the new ones go with the request.
        let many = (0..MAX_HISTORY_FILES + 2)
            .map(|index| said(MessageRole::User, "foto", vec![photo(&format!("h{index}.jpg"))]))
            .collect::<Vec<_>>();
        let messages = turn_messages(&many, "y esta?", vec![photo("nueva.jpg")]);
        let earlier = messages[..many.len()].iter().flat_map(|message| &message.attachments).map(|file| file.name.as_str()).collect::<Vec<_>>();
        assert_eq!(earlier.len(), MAX_HISTORY_FILES);
        assert_eq!(earlier.first(), Some(&"h2.jpg"));
        assert!(messages[0].attachments.is_empty() && messages[1].attachments.is_empty());
        assert_eq!(messages.last().expect("request").attachments, vec![photo("nueva.jpg")]);
    }

    #[test]
    fn an_answered_prompt_shows_the_choice_instead_of_the_choices() {
        let parts = plan_parts(&ExecutionPlan {
            plan_id: "p".into(),
            generation: 1,
            title: "Plan".into(),
            status: crate::interaction::PlanStatus::AwaitingApproval,
            steps: vec![crate::interaction::PlanStep {
                id: "a".into(),
                label: "Mover <bancos>".into(),
                operation_id: None,
                status: crate::interaction::PlanStepStatus::Pending,
            }],
        });
        let last = parts.last().expect("plan");
        assert!(last.ends_with(CONFIRMATION_CHOICES));
        let answered = answered_prompt(last, confirmation_outcome(Some(ConfirmationReply::ConfirmAll)));
        assert!(answered.contains("1. Mover &lt;bancos&gt;") && !answered.contains("Elegí Confirmar"));
        assert!(answered.ends_with("<b>Elegiste: Confirmar todos. No te vuelvo a preguntar en este pedido.</b>"));
        assert!(answered_prompt(last, confirmation_outcome(None)).ends_with("<b>Venció sin respuesta. No se aplicaron cambios.</b>"));
        // A question keeps its text and repeats the answer, escaped and short.
        let question = answered_prompt("¿Qué carpeta?", &question_outcome(Some("Ideas & <b>")));
        assert_eq!(question, "¿Qué carpeta?\n\n<b>Respuesta: Ideas &amp; &lt;b&gt;</b>");
        assert_eq!(question_outcome(Some(&"x".repeat(300))).chars().count(), "Respuesta: ".len() + 201);
    }

    #[test]
    fn decisions_and_choices_are_parsed() {
        assert_eq!(parse_confirmation_decision("Sí, confirmo!"), Some(ConfirmationReply::Confirm));
        assert_eq!(parse_confirmation_decision("Confirmar todos"), Some(ConfirmationReply::ConfirmAll));
        assert_eq!(parse_confirmation_decision("sí a todo"), Some(ConfirmationReply::ConfirmAll));
        assert_eq!(parse_confirmation_decision("Proponer otra cosa"), Some(ConfirmationReply::Propose));
        assert_eq!(parse_confirmation_decision("no"), Some(ConfirmationReply::Cancel));
        assert_eq!(parse_confirmation_decision("tal vez"), None);
        for (_, reply) in CONFIRMATION_BUTTONS {
            assert_eq!(ConfirmationReply::from_id(reply.id()), Some(reply));
        }
        let choices = vec!["Banco".to_string(), "Efectivo".to_string()];
        assert_eq!(resolve_choice_reply("2)", &choices), ("Efectivo".to_string(), Some(1)));
        assert_eq!(resolve_choice_reply("otra", &choices), ("otra".to_string(), None));
    }

    #[test]
    fn details_are_redacted_and_escaped() {
        let text = redact_detail("operationId=abc api_key: xyz en C:\\Users\\ana\\x.md");
        assert_eq!(text, "operationId=[oculto] api_key=[oculto] en [ruta privada]");
    }

    #[test]
    fn finance_requests_are_detected() {
        assert!(is_finance_request("¿Cuánto gasté en nafta?"));
        assert!(is_finance_request("anotá $ 500 de almuerzo"));
        assert!(!is_finance_request("resumí la nota de ayer"));
        // «cuenta» names the mail account here: the request goes to the
        // library, where the Gmail tools are.
        let mail = "necesito que de mi cuenta de gmail, elimines todos los email de \"Tienda Vapor\"";
        assert!(is_mail_request(mail));
        assert!(!is_finance_request(mail));
        assert!(is_mail_request("¿qué tengo en el calendario mañana?"));
        assert!(is_finance_request("pagué la cuenta de la luz"));
        assert!(!is_mail_request("pagué la cuenta de la luz"));
    }

    #[test]
    fn progress_follows_phases_and_tools() {
        let events = vec![
            BackendEvent::RoundStarted { request_id: "r".into(), round: 1 },
            BackendEvent::ToolStarted { request_id: "r".into(), tool_name: "search_library_documents".into(), round: 1 },
        ];
        assert_eq!(progress_message(&events).as_deref(), Some("<b>Leyendo la información necesaria</b> (paso 1)\nAhora: buscando en la biblioteca."));
        assert_eq!(progress_message(&[]), None);
    }

    #[test]
    fn progress_shows_the_last_note_of_the_agent() {
        let events = vec![
            BackendEvent::RoundStarted { request_id: "r".into(), round: 2 },
            BackendEvent::AssistantNote { request_id: "r".into(), text: "Busco los correos.".into() },
            BackendEvent::AssistantNote { request_id: "r".into(), text: "Voy a ver el volumen total <antes> de borrar.".into() },
            BackendEvent::ToolStarted { request_id: "r".into(), tool_name: "search_library_documents".into(), round: 2 },
        ];
        assert_eq!(
            progress_message(&events).as_deref(),
            Some("<b>Leyendo la información necesaria</b> (paso 2)\n<i>Voy a ver el volumen total &lt;antes&gt; de borrar.</i>\nAhora: buscando en la biblioteca.")
        );
        let long = vec![
            BackendEvent::RoundStarted { request_id: "r".into(), round: 1 },
            BackendEvent::AssistantNote { request_id: "r".into(), text: "x".repeat(400) },
        ];
        assert!(progress_message(&long).expect("message").ends_with("…</i>"));
    }

    #[test]
    fn the_progress_shows_a_markdown_note_as_plain_text() {
        let events = vec![
            BackendEvent::RoundStarted { request_id: "r".into(), round: 4 },
            BackendEvent::AssistantNote {
                request_id: "r".into(),
                text: "Miré la cuenta **gabmiuzzi@gmail.com**.\n\n## 1. Carpeta Spam\n| Remitente | Cant. |\n|---|---|\n| LinkedIn | 19 |".into(),
            },
        ];
        assert_eq!(
            progress_message(&events).as_deref(),
            Some("<b>Leyendo la información necesaria</b> (paso 4)\n<i>Miré la cuenta gabmiuzzi@gmail.com. 1. Carpeta Spam Remitente Cant. LinkedIn 19</i>")
        );
    }

    #[test]
    fn notes_the_progress_clipped_or_that_give_context_to_a_question_are_delivered_whole() {
        let note = |text: &str| BackendEvent::AssistantNote { request_id: "r".into(), text: text.into() };
        let round = |round: u32| BackendEvent::RoundStarted { request_id: "r".into(), round };
        let analysis = format!("## Spam\n{}", "LinkedIn (19), Reddit (17). ".repeat(20)).trim().to_string();
        let events = vec![round(1), note("Busco los correos."), note(&analysis), round(2), note("¿Borro estos?")];
        // Before a question: the clipped analysis and the note of the round that asks.
        assert_eq!(notes_to_deliver(&events, true), vec![analysis.clone(), "¿Borro estos?".to_string()]);
        // Before the answer: only what the progress clipped.
        assert_eq!(notes_to_deliver(&events, false), vec![analysis]);
        // A short status of an earlier round was read in the progress.
        assert!(notes_to_deliver(&[round(1), note("Busco los correos."), round(2)], true).is_empty());
    }

    #[test]
    fn a_long_answer_is_split_between_paragraphs_without_cutting_tags() {
        let paragraph = |index: usize| format!("**Grupo {index}**: {}", "correo de promoción ".repeat(30));
        let markdown = (0..12).map(paragraph).collect::<Vec<_>>().join("\n\n");
        let parts = telegram_html_parts(&markdown);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| part.chars().count() <= MAX_MESSAGE_HTML));
        assert!(parts.iter().all(|part| part.matches("<b>").count() == part.matches("</b>").count()));
        assert_eq!(parts.iter().map(|part| part.matches("<b>Grupo").count()).sum::<usize>(), 12);
        assert_eq!(telegram_html_parts("Hola **vos**"), vec!["Hola <b>vos</b>".to_string()]);
        assert!(telegram_html_parts("   ").is_empty());
    }

    #[test]
    fn nothing_is_cut_however_long_it_is() {
        let size = |parts: &[String]| parts.iter().all(|part| part.chars().count() <= MAX_MESSAGE_HTML);
        // A code block longer than a message: every piece keeps its fence.
        let code = format!("```json\n{}\n```", (0..600).map(|index| format!("  \"campo{index}\": {index},")).collect::<Vec<_>>().join("\n"));
        let parts = telegram_html_parts(&code);
        assert!(parts.len() > 1 && size(&parts));
        assert!(parts.iter().all(|part| part.starts_with("<pre>") && part.ends_with("</pre>")));
        assert_eq!(parts.iter().map(|part| part.matches("campo").count()).sum::<usize>(), 600);
        // One line without breaks, longer than a message.
        let line = "palabra ".repeat(2_000);
        let parts = telegram_html_parts(&line);
        assert!(parts.len() > 1 && size(&parts));
        assert_eq!(parts.iter().map(|part| part.matches("palabra").count()).sum::<usize>(), 2_000);
        // A long plan and a long confirmation are split, never clipped.
        let plan = ExecutionPlan {
            plan_id: "p".into(),
            generation: 1,
            title: "Plan".into(),
            status: crate::interaction::PlanStatus::AwaitingApproval,
            steps: (0..300)
                .map(|index| crate::interaction::PlanStep {
                    id: format!("s{index}"),
                    label: format!("Mandar a la papelera el correo número {index} de la carpeta Spam"),
                    operation_id: None,
                    status: crate::interaction::PlanStepStatus::Pending,
                })
                .collect(),
        };
        let parts = plan_parts(&plan);
        assert!(parts.len() > 1 && size(&parts));
        assert!(parts.join("\n").contains("300. Mandar a la papelera el correo número 299"));
        assert!(!parts.join("").contains('…'));
        // A long option is listed whole in the question.
        let option = "Mandar a la papelera el Spam y las promociones comerciales de Recibidos".to_string();
        let parts = question_parts("¿Qué borro?", &["Solo Spam".into(), option.clone()]);
        assert!(parts.join("").contains(&option));
        assert_eq!(question_parts("¿Qué borro?", &["Solo Spam".into()]), vec!["¿Qué borro?".to_string()]);
    }
}
