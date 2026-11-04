//! What each phase of a task reads and how its answer is read.
//!
//! The prompt files of `.agent/promps` say who each agent is; the person
//! may edit them. The format of the answers Notia reads (the plan's steps,
//! the review's verdict) lives here, in the system prompt section of each
//! phase, so an edited prompt never breaks a task.

use super::{clipped, AiTask, AiTaskPhase, PlannedStep, ReviewOutcome, StepStatus, MAX_STEPS, MAX_STEP_DETAIL_CHARS, MAX_STEP_TITLE_CHARS};
use crate::ai_actions::fold;

/// Characters of a step's report the next phases read.
const REPORT_CHARS_IN_REQUEST: usize = 4_000;
/// Characters of every report together a request carries.
const REPORTS_BUDGET_CHARS: usize = 80_000;

/// System prompt section of a phase's run.
pub fn phase_guidance(phase: AiTaskPhase) -> String {
    let common = "Esta corrida es parte de una Tarea IA de Notia: trabajás fuera del chat, sobre una tarea guardada que la persona sigue en el panel «Tareas IA». No ves la conversación donde se pidió: todo lo que necesitás está en el pedido. No crees otras tareas.";
    let specific = match phase {
        AiTaskPhase::Planner => {
            "Sos el PLANIFICADOR. Mirá lo que haga falta con herramientas de lectura y respondé SOLO con el plan, con este formato exacto:\nPASOS:\n1. Título corto del paso — qué hacer, sobre qué y cuándo está terminado\n2. …\nUn paso por línea, numerados, sin texto antes ni después.
Si el objetivo depende de otros (que respondan un correo o un WhatsApp, que alguien haga algo) o de una fecha, planificá también esos pasos: el Ejecutor puede esperar respuestas, una fecha o controlar cada tanto, y seguir cuando pase. Un paso así dice qué esperar y qué hacer con la respuesta (por ejemplo «Mandar el pedido de acceso a soporte y esperar su respuesta — seguir la conversación hasta que confirmen el acceso»).
Una pregunta a la persona es un paso del Ejecutor (le llega por Telegram y la tarea espera su respuesta); el resultado final le llega solo al terminar, así que no planifiques un paso aparte para mandarlo."
        }
        AiTaskPhase::Executor => {
            "Sos el EJECUTOR. Hacé solo el paso que te toca, con tus herramientas, hasta terminarlo. Tu respuesta final es el informe del paso: qué hiciste, sobre qué y qué quedó pendiente. Si no pudiste terminarlo, empezá el informe con «NO TERMINADO:» y explicá por qué.
Si el paso depende de algo que todavía no pasó (la respuesta a un correo que mandaste, una fecha, que alguien haga algo), no lo des por terminado ni por fallido: llamá wait_ai_task con qué esperás y terminá tu respuesta con lo que hiciste hasta ahora. Notia deja la tarea esperando sin gastar corridas y te vuelve a llamar en este mismo paso cuando llegue lo esperado, en cada control o cuando venza el plazo, con lo que pasó. Para esperar la respuesta a un correo usá kind mail con el threadId que devolvió send_gmail_message; para una respuesta por WhatsApp (un bot o el soporte de una empresa), kind whatsapp con el chat que devolvió whatsapp_send_message: Notia te llama cuando dejaron de escribir, con todo lo que mandaron; si mandan un menú con opciones, elegí la opción con whatsapp_send_message, replyTo = el id del menú y text = la opción tal cual. Así seguís la conversación (responder, insistir, volver a esperar) hasta lograr el objetivo del paso.
Para preguntarle algo a la persona (un dato, una decisión, que revise una lista) usá request_user_clarification: le llega por Telegram, si lo vinculó, y al panel, y la tarea espera su respuesta (un botón o un mensaje respondiendo a la pregunta). El resultado de la tarea le llega solo por Telegram cuando termina: un paso de «avisar» o «mandar el informe por Telegram» se cumple con tu informe."
        }
        AiTaskPhase::Review => {
            "Sos el REVIEW. Verificá el trabajo con herramientas de lectura y respondé con este formato exacto:\nVEREDICTO: aprobada | rehacer\nCOMENTARIOS:\nqué está bien y qué falta o está mal\nPASOS:\n1. Paso para corregir — qué hacer (solo si el veredicto es rehacer)\nRESULTADO:\nel resultado para la persona: qué se hizo y dónde quedó (solo si el veredicto es aprobada)"
        }
    };
    format!("{common}\n{specific}")
}

/// The request of the planner. A task planned again after its goal
/// changed shows what was already done, so only what is missing is planned.
pub fn planner_request(task: &AiTask) -> String {
    let mut text = format!("[Tarea IA «{}» · Planificador]\nObjetivo:\n{}", task.title, task.goal.trim());
    let reports = done_reports(task);
    if task.replan && !reports.is_empty() {
        text.push_str(&format!(
            "\n\nEl objetivo se corrigió después de empezar. Lo que ya se hizo:\n{reports}\n\nArmá solo los pasos que faltan para cumplir el objetivo, sin repetir lo hecho; si ya está cumplido, un solo paso que lo verifique y cierre."
        ));
    } else {
        text.push_str("\n\nArmá el plan de pasos para cumplir este objetivo.");
    }
    text
}

/// The request of the executor for the step at `index`.
pub fn executor_request(task: &AiTask, index: usize) -> String {
    let step = &task.steps[index];
    let mut text = format!(
        "[Tarea IA «{}» · Ejecutor · paso {} de {}]\nObjetivo:\n{}\n\nPlan:\n{}",
        task.title,
        index + 1,
        task.steps.len(),
        task.goal.trim(),
        plan_lines(task, Some(index))
    );
    let reports = done_reports(task);
    if !reports.is_empty() {
        text.push_str(&format!("\n\nLo que ya se hizo:\n{reports}"));
    }
    if step.round > 0 {
        if let Some(review) = task.reviews.iter().rev().find(|review| !review.approved) {
            text.push_str(&format!("\n\nLa revisión pidió rehacer:\n{}", review.feedback.trim()));
        }
    }
    if !step.notes.is_empty() {
        text.push_str(&format!("

Lo que ya hiciste en este paso, antes de esperar:
{}", super::waits::notes_for_request(&step.notes)));
    }
    if let Some(wake) = task.wake.as_deref() {
        text.push_str(&format!("

Cómo terminó la espera:
{}", wake.trim()));
    }
    if step.status == StepStatus::Failed {
        if let Some(report) = step.report.as_deref() {
            text.push_str(&format!(
                "\n\nEste paso ya se intentó y quedó así:\n{}\nNo repitas lo que ya cambió datos o mandó mensajes: verificá cómo quedó y seguí desde ahí.",
                clipped(report, REPORT_CHARS_IN_REQUEST)
            ));
        }
    }
    text.push_str(&format!(
        "\n\nTe toca el paso {}: {}\n{}\nHacé solo este paso y respondé con su informe.",
        index + 1,
        step.title,
        step.detail.trim()
    ));
    text
}

/// The request of the review.
pub fn review_request(task: &AiTask) -> String {
    let mut text = format!(
        "[Tarea IA «{}» · Review]\nObjetivo:\n{}\n\nPlan:\n{}\n\nInformes de los pasos:\n{}",
        task.title,
        task.goal.trim(),
        plan_lines(task, None),
        done_reports(task)
    );
    let previous = task
        .reviews
        .iter()
        .map(|review| format!("- Revisión de la ronda {}: {} — {}", review.round + 1, if review.approved { "aprobada" } else { "rehacer" }, clipped(&review.feedback, 1_000)))
        .collect::<Vec<_>>();
    if !previous.is_empty() {
        text.push_str(&format!("\n\nRevisiones anteriores:\n{}", previous.join("\n")));
    }
    if let Some(request) = task.review_request.as_deref().filter(|request| !request.trim().is_empty()) {
        text.push_str(&format!("\n\nTe piden revisar especialmente:\n{}", request.trim()));
    }
    text.push_str("\n\nRevisá que el objetivo esté cumplido y respondé con el veredicto.");
    text
}

fn plan_lines(task: &AiTask, current: Option<usize>) -> String {
    task.steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let mark = if Some(index) == current { "← ahora".to_string() } else { step.status.label().to_lowercase() };
            format!("{}. {} ({mark})", index + 1, step.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn done_reports(task: &AiTask) -> String {
    let mut used = 0;
    let mut lines = Vec::new();
    for (index, step) in task.steps.iter().enumerate() {
        let Some(report) = step.report.as_deref().filter(|_| step.status == StepStatus::Done) else { continue };
        let report = clipped(report, REPORT_CHARS_IN_REQUEST);
        used += report.chars().count();
        if used > REPORTS_BUDGET_CHARS {
            lines.push("(los informes siguientes no entran; leé lo que necesites con herramientas)".to_string());
            break;
        }
        lines.push(format!("Paso {} · {}:\n{report}", index + 1, step.title));
    }
    lines.join("\n\n")
}

/// The steps of a planner's answer: its numbered lines (`1.`, `1)`), or its
/// bullets when it numbered none. A planner that wrote no step does not fail
/// the task: the steps written in `goal` are the plan, or the whole goal is
/// one step.
pub fn parse_plan(answer: &str, goal: &str) -> Vec<PlannedStep> {
    let steps = list_steps(answer);
    if !steps.is_empty() {
        return steps;
    }
    let written = list_steps(goal);
    if !written.is_empty() {
        return written;
    }
    vec![PlannedStep { title: "Cumplir el objetivo".to_string(), detail: clipped(goal, MAX_STEP_DETAIL_CHARS) }]
}

fn list_steps(text: &str) -> Vec<PlannedStep> {
    let numbered = text.lines().filter_map(numbered_item).collect::<Vec<_>>();
    let items = if numbered.is_empty() {
        text.lines().filter_map(|line| line.trim().strip_prefix("- ").or_else(|| line.trim().strip_prefix("* "))).map(str::to_string).collect()
    } else {
        numbered
    };
    items.iter().map(|item| planned_step(item)).filter(|step| !step.title.is_empty()).take(MAX_STEPS).collect()
}

/// The text of a `1. …` or `1) …` line.
fn numbered_item(line: &str) -> Option<String> {
    let line = line.trim();
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

/// A step's title and detail, split at the first « — », « – », «: » or « - ».
fn planned_step(item: &str) -> PlannedStep {
    let item = item.replace("**", "");
    let item = item.trim();
    let split = [" — ", " – ", ": ", " - "].iter().filter_map(|separator| item.find(separator).map(|at| (at, separator.len()))).min();
    let (title, detail) = match split {
        Some((at, length)) => (&item[..at], &item[at + length..]),
        None => (item, ""),
    };
    PlannedStep { title: clipped(title, MAX_STEP_TITLE_CHARS), detail: clipped(detail, MAX_STEP_DETAIL_CHARS) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Verdict,
    Comments,
    Steps,
    Result,
}

/// The section a heading line opens and what follows it on the line.
fn section_heading(line: &str) -> Option<(Section, String)> {
    let trimmed = line.trim().trim_start_matches(['#', '*', ' ']).replace("**", "");
    let (head, rest) = match trimmed.split_once(':') {
        Some((head, rest)) => (head.to_string(), rest.trim().to_string()),
        None => (trimmed.clone(), String::new()),
    };
    let section = match fold(head.trim()).as_str() {
        "veredicto" => Section::Verdict,
        "comentarios" | "comentario" => Section::Comments,
        "pasos" => Section::Steps,
        "resultado" => Section::Result,
        _ => return None,
    };
    Some((section, rest))
}

/// What a review answered. Without a verdict line, a rework is read from
/// «rehacer» in the text or from the steps it lists; otherwise it approves.
pub fn parse_review(answer: &str) -> ReviewOutcome {
    let mut sections: Vec<(Section, Vec<String>)> = Vec::new();
    let mut loose = Vec::new();
    for line in answer.lines() {
        match section_heading(line) {
            Some((section, rest)) => {
                sections.push((section, Vec::new()));
                if !rest.is_empty() {
                    sections.last_mut().expect("just pushed").1.push(rest);
                }
            }
            None => match sections.last_mut() {
                Some((_, lines)) => lines.push(line.to_string()),
                None => loose.push(line.to_string()),
            },
        }
    }
    let text = |wanted: Section| {
        sections.iter().filter(|(section, _)| *section == wanted).flat_map(|(_, lines)| lines.iter().cloned()).collect::<Vec<_>>().join("\n").trim().to_string()
    };
    let verdict = fold(&text(Section::Verdict));
    let steps = list_steps(&text(Section::Steps));
    let approved = if verdict.contains("aprob") {
        true
    } else if verdict.contains("rehac") || verdict.contains("rechaz") || verdict.contains("rework") {
        false
    } else {
        !(fold(answer).contains("rehacer") || !steps.is_empty())
    };
    let comments = text(Section::Comments);
    let feedback = if comments.is_empty() && sections.is_empty() { answer.trim().to_string() } else if comments.is_empty() { loose.join("\n").trim().to_string() } else { comments };
    ReviewOutcome { approved, feedback, steps: if approved { Vec::new() } else { steps }, result: text(Section::Result) }
}

/// The executor could not finish the step.
pub fn step_unfinished(report: &str) -> bool {
    fold(report.trim_start().trim_start_matches(['*', '#', ' '])).starts_with("no terminado")
}

/// The run of a step worked but the model wrote no report: the answer is
/// only the count of its actions, so the step has no deliverable.
pub fn step_without_report(answer: &str) -> bool {
    answer.trim_start().starts_with(crate::agent::WORK_SUMMARY_START)
}

/// The Telegram message of a task that ended.
pub fn finished_message(task: &AiTask) -> String {
    match task.status {
        super::AiTaskStatus::Completed => format!(
            "**Terminé la tarea «{}».**\n\n{}",
            task.title,
            task.result.as_deref().unwrap_or("La revisión la aprobó.").trim()
        ),
        _ => format!(
            "**La tarea «{}» quedó {}.**\n\n{}\n\nLa podés reanudar desde Notia → Tareas IA o pidiéndomelo.",
            task.title,
            task.status.label().to_lowercase(),
            task.error.as_deref().unwrap_or("").trim()
        ),
    }
}

/// The Telegram message that asks for an approval.
pub fn approval_message(task: &AiTask, approval: &super::AiTaskApproval) -> String {
    let what = match approval.kind {
        super::ApprovalKind::Confirmation => "necesita tu confirmación",
        super::ApprovalKind::Plan => "necesita que apruebes un plan",
        super::ApprovalKind::Clarification => "tiene una pregunta",
    };
    let mut text = format!("**Tarea «{}» · {}:** {what}\n\n{}", task.title, approval.phase.label(), approval.title.trim());
    if !approval.detail.trim().is_empty() {
        text.push_str(&format!("\n\n{}", clipped(&approval.detail, 2_500)));
    }
    text.push_str(match approval.kind {
        super::ApprovalKind::Clarification if approval.options.is_empty() => {
            "\n\nRespondé a este mensaje (Responder) o desde Notia → Tareas IA."
        }
        super::ApprovalKind::Clarification => "\n\nElegí una opción o respondé a este mensaje con otra respuesta.",
        _ => "\n\nPara pedir otra cosa en lugar de esto, respondé a este mensaje (Responder).",
    });
    text
}
