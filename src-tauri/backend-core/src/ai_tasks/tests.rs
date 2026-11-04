use super::prompts::{executor_request, parse_plan, parse_review, phase_guidance, review_request, step_unfinished, step_without_report};
use super::tools::{list_view, new_task, resolve_task};
use super::workflow::workflow_mermaid;
use super::*;

fn task() -> AiTask {
    AiTask::new("t1".into(), "Ordenar la wiki", "Reordenar el índice del curso y encadenar nextPage/previousPage.", AiTaskOrigin::Telegram, Some(7), false, 1_000)
}

fn done(task: &mut AiTask, index: usize, report: &str) {
    task.steps[index].status = StepStatus::Done;
    task.steps[index].report = Some(report.to_string());
}

#[test]
fn the_plan_is_read_from_numbered_lines_or_bullets() {
    let steps = parse_plan("PASOS:\n1. Leer el índice — abrir Cursos/Indice.md\n2) **Reordenar** las etapas: mover la etapa 4\n3. Encadenar\n\nListo.", "");
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0], PlannedStep { title: "Leer el índice".into(), detail: "abrir Cursos/Indice.md".into() });
    assert_eq!(steps[1], PlannedStep { title: "Reordenar las etapas".into(), detail: "mover la etapa 4".into() });
    assert_eq!(steps[2].detail, "");
    let bullets = parse_plan("- Buscar - en la biblioteca\n- Escribir", "");
    assert_eq!(bullets.len(), 2);
    // A planner without steps does not fail the task: the goal's own steps,
    // or the whole goal as one step.
    let written = parse_plan("No sé cómo hacerlo.", "Trámite:\n1. Pedir la clave\n2. Avisar");
    assert_eq!(written.iter().map(|step| step.title.as_str()).collect::<Vec<_>>(), ["Pedir la clave", "Avisar"]);
    let whole = parse_plan("", "Recuperar la contraseña de Swiss Medical.");
    assert_eq!(whole, vec![PlannedStep { title: "Cumplir el objetivo".into(), detail: "Recuperar la contraseña de Swiss Medical.".into() }]);
    let many = (1..=40).map(|index| format!("{index}. Paso {index}")).collect::<Vec<_>>().join("\n");
    assert_eq!(parse_plan(&many, "").len(), MAX_STEPS);
}

#[test]
fn the_review_verdict_steps_and_result_are_read() {
    let approved = parse_review("VEREDICTO: Aprobada\nCOMENTARIOS:\nTodo encadenado.\nRESULTADO:\nEl índice quedó en 8 etapas.");
    assert!(approved.approved && approved.steps.is_empty());
    assert_eq!(approved.feedback, "Todo encadenado.");
    assert_eq!(approved.result, "El índice quedó en 8 etapas.");

    let rework = parse_review("**Veredicto:** rehacer\nComentarios: faltan 3 documentos\nPASOS:\n1. Encadenar Return — poner nextPage\n2. Revisar Datos.md");
    assert!(!rework.approved);
    assert_eq!(rework.feedback, "faltan 3 documentos");
    assert_eq!(rework.steps.len(), 2);

    // Without a verdict line: «rehacer» in the text asks for more work.
    assert!(!parse_review("Hay que rehacer el paso 2.").approved);
    let loose = parse_review("Quedó bien.");
    assert!(loose.approved && loose.feedback == "Quedó bien.");
}

#[test]
fn the_task_plans_runs_each_step_and_reviews_until_approved() {
    let mut task = task();
    assert_eq!(task.next_work(), NextWork::Plan);
    task.set_plan(parse_plan("1. A\n2. B", ""));
    assert_eq!(task.steps.iter().map(|step| step.id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    assert_eq!(task.next_work(), NextWork::Execute(0));
    done(&mut task, 0, "Hice A.");
    assert_eq!(task.next_work(), NextWork::Execute(1));
    done(&mut task, 1, "Hice B.");
    assert_eq!(task.next_work(), NextWork::Review);

    let effect = task.apply_review(parse_review("VEREDICTO: rehacer\nCOMENTARIOS: falta C\nPASOS:\n1. C"), 2_000);
    assert_eq!(effect, ReviewEffect::Rework);
    assert_eq!((task.steps[2].id.as_str(), task.steps[2].round), ("s3", 1));
    assert_eq!(task.next_work(), NextWork::Execute(2));
    let request = executor_request(&task, 2);
    assert!(request.contains("La revisión pidió rehacer:\nfalta C") && request.contains("Hice A.") && request.contains("paso 3 de 3"), "{request}");
    done(&mut task, 2, "Hice C.");
    assert_eq!(task.next_work(), NextWork::Review);
    assert_eq!(task.apply_review(parse_review("VEREDICTO: aprobada\nRESULTADO: A, B y C."), 3_000), ReviewEffect::Approved);
    assert_eq!(task.result.as_deref(), Some("A, B y C."));
    assert_eq!(task.next_work(), NextWork::Done);

    // Another review asked by the agent runs once more.
    task.review_request = Some("Mirá el paso 2".into());
    assert_eq!(task.next_work(), NextWork::Review);
    assert!(review_request(&task).contains("Te piden revisar especialmente:\nMirá el paso 2"));
}

#[test]
fn a_corrected_goal_keeps_the_steps_done_and_plans_what_is_missing() {
    let mut task = task();
    task.set_plan(parse_plan("1. A\n2. B\n3. C", ""));
    done(&mut task, 0, "Hice A.");
    task.steps[1].status = StepStatus::Waiting;
    task.status = AiTaskStatus::Waiting;
    task.apply_review(parse_review("VEREDICTO: aprobada"), 1);
    task.replan(Some("Nuevo objetivo con la URL."), false);
    assert_eq!((task.goal.as_str(), task.status, task.steps.len()), ("Nuevo objetivo con la URL.", AiTaskStatus::Planning, 1));
    assert_eq!(task.next_work(), NextWork::Plan);
    let request = super::prompts::planner_request(&task);
    assert!(request.contains("Nuevo objetivo con la URL.") && request.contains("Hice A.") && request.contains("solo los pasos que faltan"), "{request}");
    task.set_plan(parse_plan("1. D", ""));
    assert_eq!(task.steps.iter().map(|step| (step.id.as_str(), step.round)).collect::<Vec<_>>(), [("s1", 0), ("s2", 1)]);
    assert_eq!(task.next_work(), NextWork::Execute(1));
    done(&mut task, 1, "Hice D.");
    assert_eq!(task.next_work(), NextWork::Review, "the old approval does not cover the new steps");

    task.replan(None, true);
    assert!(task.steps.is_empty() && task.reviews.is_empty() && !task.replan);
    assert_eq!(task.goal, "Nuevo objetivo con la URL.");
    assert!(super::prompts::planner_request(&task).contains("Armá el plan de pasos"));
}

#[test]
fn a_rework_without_steps_adds_one_and_too_many_reworks_stop() {
    let mut task = task();
    task.set_plan(vec![PlannedStep { title: "A".into(), detail: String::new() }]);
    done(&mut task, 0, "ok");
    assert_eq!(task.apply_review(parse_review("VEREDICTO: rehacer\nCOMENTARIOS: mal formato"), 1), ReviewEffect::Rework);
    assert_eq!(task.steps[1].title, "Corregir lo que pidió la revisión");
    assert_eq!(task.steps[1].detail, "mal formato");
    for round in 1..MAX_REWORK_REVIEWS {
        let last = task.steps.len() - 1;
        done(&mut task, last, "ok");
        let effect = task.apply_review(parse_review("VEREDICTO: rehacer"), round as i64);
        assert_eq!(effect, if round + 1 == MAX_REWORK_REVIEWS { ReviewEffect::Exhausted } else { ReviewEffect::Rework });
    }
}

#[test]
fn the_workflow_draws_every_phase_with_its_state() {
    let mut task = task();
    assert!(workflow_mermaid(&task).contains("class plan running"));
    task.set_plan(parse_plan("1. Leer \"índice\"\n2. B", ""));
    task.status = AiTaskStatus::Running;
    task.phase = Some(AiTaskPhase::Executor);
    done(&mut task, 0, "ok");
    task.steps[1].status = StepStatus::Running;
    task.status = AiTaskStatus::AwaitingApproval;
    let graph = workflow_mermaid(&task);
    assert!(graph.starts_with("flowchart TD"));
    assert!(graph.contains("s1[\"1. Leer #quot;índice#quot;\"]") && graph.contains("plan --> s1") && graph.contains("s1 --> s2"));
    assert!(graph.contains("class s2 waiting") && graph.contains("class start,plan,s1 done"), "{graph}");

    done(&mut task, 1, "ok");
    task.apply_review(parse_review("VEREDICTO: rehacer\nPASOS:\n1. C"), 5);
    done(&mut task, 2, "ok");
    task.apply_review(parse_review("VEREDICTO: aprobada"), 6);
    task.status = AiTaskStatus::Completed;
    let graph = workflow_mermaid(&task);
    assert!(graph.contains("r1{\"Review 1\"}") && graph.contains("s2 --> r1") && graph.contains("r1 -->|rehacer| s3"), "{graph}");
    assert!(graph.contains("r2 -->|aprobada| fin") && graph.contains("fin([\"Resultado\"])"), "{graph}");
    assert!(graph.contains("class r1 rework"));
}

#[test]
fn tools_read_their_arguments_and_find_tasks() {
    assert!(new_task(&serde_json::json!({ "title": "x" })).unwrap_err().message.contains("goal"));
    let created = new_task(&serde_json::json!({ "title": " Ordenar ", "goal": "Todo" })).expect("task");
    assert_eq!(created.title, "Ordenar");
    let tasks = vec![task(), AiTask { id: "t2".into(), title: "Otra".into(), ..task() }];
    assert_eq!(resolve_task(&tasks, &serde_json::json!({ "task": "ORDENAR LA WIKI" })).expect("by title").id, "t1");
    assert_eq!(resolve_task(&tasks, &serde_json::json!({ "task": "t2" })).expect("by id").id, "t2");
    assert!(resolve_task(&tasks, &serde_json::json!({ "task": "nada" })).is_err());
    assert_eq!(list_view(&tasks)["tasks"].as_array().map(Vec::len), Some(2));
}

#[test]
fn each_phase_gets_its_format_and_reports_are_read() {
    assert!(phase_guidance(AiTaskPhase::Planner).contains("PASOS:"));
    assert!(phase_guidance(AiTaskPhase::Review).contains("VEREDICTO: aprobada | rehacer"));
    assert!(phase_guidance(AiTaskPhase::Executor).contains("NO TERMINADO:"));
    assert!(step_unfinished("**NO TERMINADO:** falta el permiso"));
    assert!(!step_unfinished("Terminé el paso."));
    assert!(step_without_report("El modelo no redactó la respuesta final, pero trabajé en el pedido: 2 acciones con herramientas."));
    assert!(!step_without_report("Informe: concilié 14 movimientos."));
    assert_eq!(AiTaskStatus::from_id("awaiting-approval"), Some(AiTaskStatus::AwaitingApproval));
    assert!(AiTaskPhase::ALL.iter().all(|phase| phase.default_prompt().starts_with('#')));
}

#[test]
fn the_thinking_of_a_run_joins_its_fragments_round_by_round() {
    use crate::events::BackendEvent;
    use crate::protocol::BackendEventEnvelope;
    let mut task = task();
    task.set_plan(vec![PlannedStep { title: "Leer".into(), detail: String::new() }, PlannedStep { title: "Escribir".into(), detail: String::new() }]);
    let step_id = task.steps[1].id.clone();
    let event = |sequence: u64, event: BackendEvent| BackendEventEnvelope {
        protocol_version: Default::default(),
        request_id: "ai-task-1".into(),
        sequence,
        event,
    };
    let thinking_of = |summary: &str| BackendEvent::ThinkingSummary { request_id: "ai-task-1".into(), summary: summary.into() };
    let events = vec![
        event(1, BackendEvent::RoundStarted { request_id: "ai-task-1".into(), round: 1 }),
        event(2, thinking_of("Leo el ")),
        event(3, thinking_of("índice.")),
        event(4, BackendEvent::AssistantDelta { request_id: "ai-task-1".into(), delta: "no".into() }),
        event(5, BackendEvent::RoundStarted { request_id: "ai-task-1".into(), round: 2 }),
        event(6, thinking_of("Ahora escribo.")),
    ];
    let view = views::thinking(&task, "ai-task-1", AiTaskPhase::Executor, Some(step_id.clone()), &events);
    assert_eq!(view.text, "Leo el índice.

Ahora escribo.");
    assert_eq!((view.sequence, view.label.as_str(), view.step_id), (6, "Ejecutor · paso 2 · pensando", Some(step_id)));
    let long = "a".repeat(views::MAX_THINKING_CHARS + 50);
    let view = views::thinking(&task, "ai-task-1", AiTaskPhase::Planner, None, &[event(1, thinking_of(&long))]);
    assert_eq!(view.text.chars().count(), views::MAX_THINKING_CHARS);
    assert!(view.text.starts_with('…'));
    assert_eq!(view.label, "Planificador · pensando");
}

fn zone() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::fixed(jiff::tz::offset(-3))
}

#[test]
fn a_wait_is_read_from_the_tool_and_refuses_what_cannot_wait() {
    use super::waits::{wait_request, WaitFor, MAIL_POLL_MS};
    use serde_json::json;
    let accounts = vec!["yo@gmail.com".to_string()];
    let now = 1_000_000;
    let mail = wait_request(&json!({ "reason": "Respuesta de soporte", "kind": "mail", "threadId": "t-9" }), "s2", &accounts, &zone(), now).expect("mail");
    assert_eq!(mail.wait_for, WaitFor::Mail { account: "yo@gmail.com".into(), thread_id: Some("t-9".into()), query: None });
    assert_eq!((mail.wake_at_ms, mail.deadline_ms), (now + MAIL_POLL_MS, now + 72 * 3_600_000));
    let whatsapp = wait_request(&serde_json::json!({ "reason": "Respuesta de Movistar", "kind": "whatsapp", "chat": "Movistar" }), "s1", &[], &zone(), now).expect("whatsapp");
    assert_eq!(whatsapp.wait_for, WaitFor::WhatsApp { chat: "Movistar".into(), name: String::new(), since_ms: now });
    assert_eq!(whatsapp.wake_at_ms, now + super::waits::WHATSAPP_POLL_MS);
    assert_eq!(super::waits::poll_ms(&whatsapp.wait_for), 60_000);
    // A message that arrives makes the adapter look before the next poll.
    assert_eq!(super::waits::due(&whatsapp, &zone(), now + 1), super::waits::Due::LookAtWhatsApp);
    assert_eq!(super::waits::due(&mail, &zone(), now + 1), super::waits::Due::NotYet);
    assert!(wait_request(&serde_json::json!({ "reason": "x", "kind": "whatsapp" }), "s1", &[], &zone(), now).is_err());
    let saved = serde_json::to_value(&whatsapp.wait_for).expect("json");
    assert_eq!(saved["kind"], "whatsapp");
    assert_eq!(saved["sinceMs"], now);
    let error = |arguments: serde_json::Value, accounts: &[String]| wait_request(&arguments, "s2", accounts, &zone(), now).expect_err("refused").message;
    assert!(error(json!({ "reason": "x", "kind": "mail" }), &accounts).contains("threadId"));
    assert!(error(json!({ "reason": "x", "kind": "mail", "query": "from:a" }), &[]).contains("No hay cuentas"));
    let two = vec!["a@x.com".to_string(), "b@x.com".to_string()];
    assert!(error(json!({ "reason": "x", "kind": "mail", "query": "from:a" }), &two).contains("Falta account"));
    assert!(error(json!({ "reason": "x", "kind": "mail", "query": "from:a", "account": "c@x.com" }), &two).contains("no está conectada"));
    assert!(error(json!({ "reason": "x", "kind": "check", "checkEveryMinutes": 2 }), &accounts).contains("al menos 10"));
    assert!(error(json!({ "reason": "x", "kind": "time", "until": "1969-12-31T00:00" }), &accounts).contains("ya pasó"));
    assert!(error(json!({ "reason": "x", "kind": "mail", "query": "q", "maxWaitHours": 9999 }), &accounts).contains("30 días"));
    assert!(error(json!({ "reason": "x", "kind": "pronto" }), &accounts).contains("no existe"));

    // A local moment is read in the library's zone.
    let today = "2026-11-02T12:00:00Z".parse::<jiff::Timestamp>().expect("today").as_millisecond();
    let time = wait_request(&json!({ "reason": "Que abra la oficina", "kind": "time", "until": "2026-11-03 10:00" }), "s2", &accounts, &zone(), today).expect("time");
    let expected = "2026-11-03T13:00:00Z".parse::<jiff::Timestamp>().expect("utc").as_millisecond();
    assert_eq!((time.wake_at_ms, time.deadline_ms), (expected, expected));
    let check = wait_request(&json!({ "reason": "Que muevan el ticket", "kind": "check", "checkEveryMinutes": 30, "maxWaitHours": 1 }), "s2", &accounts, &zone(), now).expect("check");
    assert_eq!((check.wake_at_ms, check.deadline_ms), (now + 30 * 60_000, now + 3_600_000));
}

#[test]
fn a_waiting_step_wakes_with_what_happened_and_the_executor_reads_it() {
    use super::waits::{begin_wait, due, mail_note, new_messages, wait_request, wake, Due};
    use serde_json::json;
    let mut task = task();
    task.set_plan(parse_plan("1. Pedir acceso a soporte\n2. Avisar", ""));
    let now: i64 = 5_000_000;
    let wait = wait_request(&json!({ "reason": "Respuesta de soporte", "kind": "mail", "threadId": "t-9" }), "s1", &["yo@gmail.com".to_string()], &zone(), now).expect("wait");
    task.wait = Some(wait.clone());
    begin_wait(&mut task, 0, "Mandé el pedido de acceso a soporte@empresa.com.");
    assert_eq!((task.status, task.steps[0].status, task.phase), (AiTaskStatus::Waiting, StepStatus::Waiting, None));
    assert!(task.status.is_active() && task.status.is_resumable());
    assert_eq!(task.next_work(), NextWork::Execute(0));

    assert_eq!(due(&wait, &zone(), now + 1_000), Due::NotYet);
    assert_eq!(due(&wait, &zone(), wait.wake_at_ms), Due::LookAtMail);
    // Only what arrived after the wait began and was not sent by the account.
    let at = |offset: i64| (now + offset).to_string();
    let messages = vec![
        json!({ "id": "m1", "internalDate": at(-1), "labelIds": ["INBOX"] }),
        json!({ "id": "m2", "internalDate": at(10), "labelIds": ["SENT"] }),
        json!({ "id": "m3", "internalDate": at(20), "labelIds": ["INBOX", "UNREAD"] }),
    ];
    let new = new_messages(&messages, now);
    assert_eq!(new.iter().map(|message| message["id"].as_str().unwrap_or_default()).collect::<Vec<_>>(), ["m3"]);
    let note = mail_note("yo@gmail.com", &[json!({ "id": "m3", "threadId": "t-9", "from": "Soporte", "subject": "Re: acceso", "date": "hoy", "snippet": "Ya tenés acceso" })]);
    assert!(note.contains("id m3") && note.contains("read_gmail_message"), "{note}");

    wake(&mut task, &note);
    assert_eq!((task.status, task.steps[0].status, task.wait.is_none()), (AiTaskStatus::Running, StepStatus::Pending, true));
    let request = executor_request(&task, 0);
    assert!(request.contains("Mandé el pedido de acceso"), "{request}");
    assert!(request.contains("Esperabas: Respuesta de soporte") && request.contains("Ya tenés acceso"), "{request}");
}

#[test]
fn a_time_or_check_wait_wakes_on_its_own_and_at_the_deadline() {
    use super::waits::{due, wait_request, Due};
    use serde_json::json;
    let now = 1_000_000;
    let accounts = Vec::new();
    let time = wait_request(&json!({ "reason": "Mañana", "kind": "time", "afterMinutes": 5 }), "s1", &accounts, &zone(), now).expect("time");
    assert!(matches!(due(&time, &zone(), now + 5 * 60_000), Due::Wake(note) if note.contains("Llegó el momento")));
    let check = wait_request(&json!({ "reason": "Ticket", "kind": "check", "checkEveryMinutes": 10, "maxWaitHours": 1 }), "s1", &accounts, &zone(), now).expect("check");
    assert!(matches!(due(&check, &zone(), now + 10 * 60_000), Due::Wake(note) if note.contains("Control de la espera")));
    assert!(matches!(due(&check, &zone(), now + 3_600_000), Due::Wake(note) if note.contains("Se cumplió el plazo")));
}

#[test]
fn only_the_executor_of_a_task_may_wait() {
    use super::tools::{phase_tools, without_task_wait};
    let tool = |name: &str| crate::ToolDefinition {
        name: name.into(),
        description: String::new(),
        input_schema: serde_json::json!({ "type": "object" }),
        scopes: Vec::new(),
        read_only: true,
        requires_confirmation: false,
    };
    let names = |tools: Vec<crate::ToolDefinition>| tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();
    let all = || vec![tool("wait_ai_task"), tool("get_ai_task")];
    assert_eq!(names(phase_tools(all(), AiTaskPhase::Executor, true)), ["wait_ai_task", "get_ai_task"]);
    assert_eq!(names(phase_tools(all(), AiTaskPhase::Planner, true)), ["get_ai_task"]);
    assert_eq!(names(phase_tools(all(), AiTaskPhase::Review, true)), ["get_ai_task"]);
    assert_eq!(names(without_task_wait(all())), ["get_ai_task"]);
}

#[test]
fn the_panel_says_what_a_waiting_task_waits_for() {
    use super::waits::{begin_wait, wait_request};
    use serde_json::json;
    let mut task = task();
    task.set_plan(parse_plan("1. Pedir acceso\n2. Avisar", ""));
    let now = 1_000_000;
    task.wait = Some(wait_request(&json!({ "reason": "Respuesta de soporte", "kind": "check", "checkEveryMinutes": 120 }), "s1", &[], &zone(), now).expect("wait"));
    begin_wait(&mut task, 0, "Pedí el acceso.");
    let detail = views::detail(&task, &zone(), now);
    let wait = detail.wait.expect("wait view");
    assert_eq!((wait.reason.as_str(), wait.kind_label.as_str()), ("Respuesta de soporte", "Controla cada 2 horas"));
    assert_eq!(detail.card.phase_label, "Paso 1 de 2 · Respuesta de soporte");
    assert_eq!((detail.resume_label.as_str(), detail.expires_label.as_str()), ("Seguir sin esperar", "No se borra mientras espera"));
    assert_eq!(detail.steps[0].notes, ["Pedí el acceso."]);
    assert!(workflow::workflow_mermaid(&task).contains("waiting"));
}
