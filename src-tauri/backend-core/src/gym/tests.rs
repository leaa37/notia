use std::collections::BTreeSet;

use super::body::parse_body_svg;
use super::change::{apply_exercise_edit, plan_change, saved_equipment, Clock, ExerciseEdit, GymMutation, SetField, StoreOp, WatchSession};
use super::markdown::{parse_equipment, parse_exercise, render_equipment, render_exercise};
use super::view::{build_view, exercise_detail, GymQuery, Screen};
use super::*;

const EXERCISE: &str = "---\nid: bench_press\ngrupo: pecho\nregistro: peso-reps\nkcalPorMinuto: 6\nprincipales: Pecho superior, Pecho inferior\nsecundarios: Tríceps, Deltoide anterior\nequipamiento: barbell, flat_bench\nvideo: Press de banca con barra.mp4\nalias: Bench Press\ncreatedAt: 1790881753884\ncontexto: \"#Personal\"\n---\n# Press de banca con barra\n\n![Press de banca con barra](data:image/png;base64,AAAA)\n\n## Cómo se hace\n\n1. Acostate en el banco.\n2. Bajá la barra al pecho.\n";

fn catalog() -> Catalog {
    let exercise = |id: &str, name: &str, group: &str, tracking: Tracking, primary: &[&str], secondary: &[&str], equipment: &[&str]| Exercise {
        id: id.into(),
        name: name.into(),
        group: group.into(),
        tracking,
        kcal_per_min: 6.0,
        primary: primary.iter().map(|value| value.to_string()).collect(),
        secondary: secondary.iter().map(|value| value.to_string()).collect(),
        equipment: equipment.iter().map(|value| value.to_string()).collect(),
        path: format!("Gym/exercises/{name}.md"),
        ..Exercise::default()
    };
    let equipment = |id: &str, name: &str, category: &str| Equipment {
        id: id.into(),
        name: name.into(),
        category: category.into(),
        path: format!("Gym/equipment/{name}.md"),
        ..Equipment::default()
    };
    Catalog {
        exercises: vec![
            exercise("bench_press", "Press de banca con barra", "pecho", Tracking::WeightReps, &["upperChest", "lowerChest"], &["triceps"], &["barbell", "flat_bench"]),
            exercise("push_ups", "Flexiones de brazos", "pecho", Tracking::Reps, &["lowerChest"], &["triceps"], &[]),
            exercise("plank", "Plancha", "core", Tracking::Time, &["abs"], &[], &[]),
            exercise("dumbbell_curl", "Curl con mancuernas", "brazos", Tracking::WeightReps, &["biceps"], &["forearms"], &["dumbbell"]),
        ],
        equipment: vec![
            equipment("barbell", "Barra y discos", "libres"),
            equipment("flat_bench", "Banco plano", "estructuras"),
            equipment("dumbbell", "Mancuernas", "libres"),
            equipment("pull_up_bar", "Barra de dominadas", "estructuras"),
            equipment("ab_wheel", "Rueda abdominal", "accesorios"),
        ],
    }
}

fn apply(data: &mut GymData, catalog: &Catalog, mutation: GymMutation, now_ms: i64, today: &str) -> Option<String> {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let mut new_id = || format!("id-{}", COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut clock = Clock { now_ms, today: today.to_string(), new_id: &mut new_id };
    let change = plan_change(data, catalog, &mutation, &mut clock).expect("change");
    for op in change.ops {
        match op {
            StoreOp::PutRoutine(routine) => match data.routines.iter_mut().find(|existing| existing.id == routine.id) {
                Some(existing) => *existing = routine,
                None => data.routines.push(routine),
            },
            StoreOp::DeleteRoutine(id) => data.routines.retain(|routine| routine.id != id),
            StoreOp::PutOwned(owned) => data.owned = owned.into_iter().collect(),
            StoreOp::PutSession(session) => data.session = session,
            StoreOp::PutWorkout(workout) => match data.workouts.iter_mut().find(|existing| existing.id == workout.id) {
                Some(existing) => *existing = workout,
                None => data.workouts.push(workout),
            },
            StoreOp::DeleteWorkout(id) => data.workouts.retain(|workout| workout.id != id),
        }
    }
    change.routine_id
}

#[test]
fn an_exercise_file_round_trips_keeping_its_image_and_note_lines() {
    let exercise = parse_exercise("Gym/exercises/Press.md", EXERCISE, true).expect("exercise");
    assert_eq!(exercise.id, "bench_press");
    assert_eq!(exercise.primary, ["upperChest", "lowerChest"]);
    assert_eq!(exercise.secondary, ["triceps", "frontDelt"]);
    assert_eq!(exercise.equipment, ["barbell", "flat_bench"]);
    assert_eq!(exercise.steps, ["Acostate en el banco.", "Bajá la barra al pecho."]);
    assert_eq!(exercise.image.as_deref(), Some("data:image/png;base64,AAAA"));
    assert_eq!(exercise.video.as_deref(), Some("Press de banca con barra.mp4"));
    assert_eq!(exercise.extra_front, [("createdAt".to_string(), "1790881753884".to_string()), ("contexto".to_string(), "#Personal".to_string())]);
    let rendered = render_exercise(&exercise);
    assert!(rendered.contains("contexto: \"#Personal\"\n"));
    assert_eq!(parse_exercise("Gym/exercises/Press.md", &rendered, true).as_ref(), Some(&exercise));
    // Las listas no cargan la imagen.
    let light = parse_exercise("Gym/exercises/Press.md", EXERCISE, false).expect("exercise");
    assert!(light.has_image && light.image.is_none());
    // Un archivo sin id no es un ejercicio.
    assert!(parse_exercise("Gym/exercises/Nota.md", "# Nota\n", false).is_none());

    let equipment = parse_equipment("Gym/equipment/Kettlebell.md", "---\nid: propio_kettlebell\ncategoria: libres\norigen: propio\n---\n# Kettlebell 16 kg\n", true).expect("equipment");
    assert!(equipment.custom && equipment.category == "libres" && equipment.name == "Kettlebell 16 kg");
    assert_eq!(parse_equipment(&equipment.path, &render_equipment(&equipment), true), Some(equipment));
}

/// A video named after the id belongs to the exercise without `video:` in
/// its file, and rewriting the file does not add it.
#[test]
fn a_video_named_after_the_id_needs_no_video_line() {
    let names = ["bench_press.MOV", "Press.mp4", "bench_press.webm", "notas.md", "bench_press.mp4", "bench_press_2.mp4", ".mp4"];
    assert_eq!(video_by_id("bench_press", names).as_deref(), Some("bench_press.mp4"));
    assert_eq!(video_by_id("BENCH_PRESS", ["bench_press.webm", "bench_press.MOV"]).as_deref(), Some("bench_press.webm"));
    assert_eq!(video_by_id("bench_press", ["bench_press.MOV"]).as_deref(), Some("bench_press.MOV"));
    assert_eq!(video_by_id("bench_press", ["bench_press.gif", "bench_press.md", "bench.mp4", "press_bench_press.mp4"]), None);
    assert_eq!(video_by_id("", [".mp4"]), None);
    assert!(is_video_name("Plancha.WebM") && !is_video_name(".mp4") && !is_video_name("Plancha.gif"));

    let mut exercise = parse_exercise("Gym/exercises/Press.md", EXERCISE, false).expect("exercise");
    exercise.video = Some("bench_press.mp4".into());
    let rendered = render_exercise(&exercise);
    assert!(!rendered.contains("video:"), "{rendered}");
    assert_eq!(parse_exercise("Gym/exercises/Press.md", &rendered, false).expect("exercise").video, None);
    // A video with another name still needs its line.
    exercise.video = Some("Press de banca.mp4".into());
    assert!(render_exercise(&exercise).contains("video: Press de banca.mp4\n"));
}

#[test]
fn the_body_is_read_as_silhouette_outline_and_muscles() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 232 523"><path class="body" fill="#E8E8F5" d="M1 1Z"/><path class="body" fill="#72728D" d="M2 2Z"/><path class="muscle" data-muscle="Upper Chest" fill="#CBCBE0" d="M3 3Z"/><path data-muscle="Rotator Cuff" d="M4 4Z"/></svg>"##;
    let figure = parse_body_svg(svg).expect("figure");
    assert_eq!(figure.view_box, "0 0 232 523");
    assert_eq!(figure.silhouette, ["M1 1Z"]);
    assert_eq!(figure.outline, ["M2 2Z"]);
    assert_eq!(figure.muscles.len(), 1);
    assert_eq!(figure.muscles[0].muscle, "upperChest");
}

#[test]
fn a_routine_is_edited_trained_and_logged() {
    let catalog = catalog();
    let mut data = GymData::default();
    let today = "2026-10-01";
    let routine = apply(&mut data, &catalog, GymMutation::CreateRoutine, 1_000, today).expect("routine");
    apply(&mut data, &catalog, GymMutation::RenameRoutine { routine_id: routine.clone(), name: "  Pecho   y tríceps ".into() }, 1_000, today);
    apply(&mut data, &catalog, GymMutation::ToggleDay { routine_id: routine.clone(), day: 2 }, 1_000, today);
    apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: routine.clone(), exercise_id: "bench_press".into() }, 1_000, today);
    apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: routine.clone(), exercise_id: "plank".into() }, 1_000, today);
    let bench = data.routines[0].items[0].id.clone();
    apply(&mut data, &catalog, GymMutation::SetValue { routine_id: routine.clone(), item_id: bench.clone(), index: 0, field: SetField::Weight, value: "60,5".into() }, 1_000, today);
    assert_eq!(data.routines[0].name, "Pecho y tríceps");
    assert_eq!(data.routines[0].days, [2]);
    assert_eq!(data.routines[0].items[0].sets[0], SetPlan { weight: Some(60.5), reps: Some(10.0) });
    assert_eq!(data.routines[0].items[1].sets[0].reps, Some(30.0), "a timed exercise starts with 30 s");

    // Marcar una serie arranca la sesión y el descanso.
    apply(&mut data, &catalog, GymMutation::ToggleSetDone { routine_id: routine.clone(), item_id: bench.clone(), index: 0 }, 10_000, today);
    let session = data.session.clone().expect("session");
    assert_eq!(session.status, SessionStatus::Running);
    assert_eq!(session.rest_end_ms, Some(10_000 + 90_000));
    apply(&mut data, &catalog, GymMutation::PauseSession, 40_000, today);
    let paused = data.session.clone().expect("session");
    assert_eq!((paused.acc_ms, paused.rest_left_ms), (30_000, Some(60_000)));
    apply(&mut data, &catalog, GymMutation::ResumeSession, 100_000, today);
    assert_eq!(data.session.as_ref().and_then(|session| session.rest_end_ms), Some(160_000));
    apply(&mut data, &catalog, GymMutation::ToggleSetDone { routine_id: routine.clone(), item_id: bench.clone(), index: 1 }, 400_000, today);
    apply(&mut data, &catalog, GymMutation::FinishSession, 1_330_000, today);
    let workout = data.workouts.last().expect("workout");
    assert_eq!((workout.sets, workout.minutes, workout.date.as_str()), (2, 21.0, today));
    assert_eq!(workout.volume, 605.0, "only the first set has a weight");
    assert_eq!(data.session.as_ref().map(|session| session.status), Some(SessionStatus::Done));
    // Al abrir Gimnasio con un entrenamiento terminado no hay nada que retomar.
    let opened = build_view(&data, &catalog, &GymQuery { resume: true, ..GymQuery::default() }, parse_date(today).expect("date"), 1_400_000);
    assert_eq!(opened.screen, Screen::Panel);
    // Una serie marcada después de terminar no arranca otro entrenamiento encima.
    let mut clock_ids = 0;
    let mut new_id = || {
        clock_ids += 1;
        format!("t-{clock_ids}")
    };
    let mut clock = Clock { now_ms: 1_400_000, today: today.into(), new_id: &mut new_id };
    let late = GymMutation::ToggleSetDone { routine_id: routine.clone(), item_id: bench.clone(), index: 2 };
    assert!(plan_change(&data, &catalog, &late, &mut clock).expect_err("finished").message.contains("Empezar de nuevo"));

    // Un entrenamiento en curso se retoma al abrir, en su pantalla, mientras tenga actividad reciente.
    apply(&mut data, &catalog, GymMutation::RestartSession, 1_500_000, today);
    apply(&mut data, &catalog, GymMutation::ToggleSetDone { routine_id: routine.clone(), item_id: bench.clone(), index: 0 }, 1_600_000, today);
    let resumed = build_view(&data, &catalog, &GymQuery { resume: true, ..GymQuery::default() }, parse_date(today).expect("date"), 1_700_000);
    assert_eq!((resumed.screen, resumed.routine_id.as_deref()), (Screen::Entrenar, Some(routine.as_str())));
    assert_eq!(resumed.training.as_ref().map(|training| (training.status.as_str(), training.done_sets)), Some(("running", 1)));
    let session = data.session.clone().expect("session");
    assert!(session.resumable(1_600_000 + RESUME_WINDOW_MS - 1));
    assert!(!session.resumable(1_600_000 + RESUME_WINDOW_MS), "a forgotten workout does not take over the app");
    apply(&mut data, &catalog, GymMutation::FinishSession, 1_800_000, today);

    // Otra rutina no pisa una sesión en curso.
    let other = apply(&mut data, &catalog, GymMutation::DuplicateRoutine { routine_id: routine.clone() }, 2_000_000, today).expect("copy");
    apply(&mut data, &catalog, GymMutation::StartSession { routine_id: routine.clone() }, 2_000_000, today);
    let mut clock_ids = 0;
    let mut new_id = || {
        clock_ids += 1;
        format!("x-{clock_ids}")
    };
    let mut clock = Clock { now_ms: 2_000_000, today: today.into(), new_id: &mut new_id };
    let refused = plan_change(&data, &catalog, &GymMutation::StartSession { routine_id: other.clone() }, &mut clock).expect_err("busy");
    assert!(refused.message.contains("Pecho y tríceps"));
    assert_eq!(data.routines[1].name, "Pecho y tríceps (copia)");
    assert!(data.routines[1].items.iter().all(|item| !data.routines[0].items.iter().any(|original| original.id == item.id)));
}

#[test]
fn the_panel_counts_the_week_and_the_muscles() {
    let catalog = catalog();
    let mut data = GymData::default();
    let routine = apply(&mut data, &catalog, GymMutation::CreateRoutine, 0, "2026-10-01").expect("routine");
    apply(&mut data, &catalog, GymMutation::ToggleDay { routine_id: routine.clone(), day: 0 }, 0, "2026-10-01");
    apply(&mut data, &catalog, GymMutation::ToggleDay { routine_id: routine.clone(), day: 3 }, 0, "2026-10-01");
    apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: routine.clone(), exercise_id: "bench_press".into() }, 0, "2026-10-01");
    let workout = |date: &str, ended_ms: i64| Workout {
        id: format!("w-{date}"),
        date: date.into(),
        routine_id: routine.clone(),
        routine_name: "Nueva rutina".into(),
        color: "azul".into(),
        ended_ms,
        minutes: 50.0,
        kcal: 300.0,
        sets: 3,
        volume: 1800.0,
        exercises: vec![WorkoutExercise { exercise: "bench_press".into(), sets: vec![SetPlan::default(); 3] }],
        watch_id: None,
        avg_bpm: None,
        max_bpm: None,
    };
    // Jueves 1 de octubre de 2026; el lunes 28 de septiembre entrenó, y las dos semanas anteriores también dos veces.
    let now = 1_790_900_000_000; // 2026-10-01 ~ 22:13 UTC
    data.workouts = vec![
        workout("2026-09-14", now - 17 * 86_400_000),
        workout("2026-09-17", now - 14 * 86_400_000),
        workout("2026-09-21", now - 10 * 86_400_000),
        workout("2026-09-24", now - 7 * 86_400_000),
        workout("2026-09-28", now - 3 * 86_400_000 + 3_600_000),
    ];
    let today = parse_date("2026-10-01").expect("date");
    let view = build_view(&data, &catalog, &GymQuery::default(), today, now);
    let panel = view.panel.expect("panel");
    assert_eq!(panel.week_label, "Semana del 28 de septiembre al 4 de octubre");
    assert_eq!(panel.next_label, "Hoy toca Nueva rutina");
    assert!(panel.next_is_today);
    let stat = |key: &str| panel.stats.iter().find(|stat| stat.key == key).cloned().expect("stat");
    assert_eq!(stat("streak").value, "2 semanas");
    assert_eq!(stat("week").value, "1 de 2");
    assert_eq!(stat("week").sub, "Falta: Nueva rutina.");
    assert_eq!(panel.muscle_states.get("upperChest").map(String::as_str), Some("rec"), "trained 71 h ago");
    assert_eq!(panel.muscle_states.get("biceps").map(String::as_str), Some("none"));
    assert_eq!(panel.weeks.len(), 24);
    assert!(panel.day.has && panel.day.date.starts_with("Lunes 28 de septiembre"));
    assert_eq!(panel.bars.len(), 14);

    // A rest day, or a day already trained, recommends nothing for today.
    let friday = build_view(&data, &catalog, &GymQuery::default(), parse_date("2026-10-02").expect("date"), now).panel.expect("panel");
    assert!(!friday.next_is_today);
    assert!(friday.next_label.starts_with("Hoy es día de descanso"), "{}", friday.next_label);
    data.workouts.push(workout("2026-10-01", now));
    let trained = build_view(&data, &catalog, &GymQuery::default(), today, now).panel.expect("panel");
    assert!(!trained.next_is_today);
    assert!(trained.next_label.starts_with("Hoy ya entrenaste"), "{}", trained.next_label);
}

#[test]
fn the_library_and_the_equipment_follow_what_the_person_owns() {
    let catalog = catalog();
    let mut data = GymData::default();
    let routine = apply(&mut data, &catalog, GymMutation::CreateRoutine, 0, "2026-10-01").expect("routine");
    apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: routine.clone(), exercise_id: "push_ups".into() }, 0, "2026-10-01");
    apply(&mut data, &catalog, GymMutation::ApplyPreset { preset: "casa".into() }, 0, "2026-10-01");
    assert_eq!(data.owned, ["ab_wheel", "dumbbell", "flat_bench", "pull_up_bar"].iter().map(|id| id.to_string()).collect::<BTreeSet<_>>());
    let today = parse_date("2026-10-01").expect("date");
    let query = GymQuery { screen: Screen::Editar, routine_id: Some(routine.clone()), ..GymQuery::default() };
    let view = build_view(&data, &catalog, &query, today, 0);
    let library = view.library.expect("library");
    assert_eq!(library.hidden, 1, "the bench press needs a barbell");
    assert!(library.rows.iter().any(|row| row.id == "push_ups" && row.added));
    let search = GymQuery { search: "barra".into(), only_mine: false, ..query.clone() };
    let found = build_view(&data, &catalog, &search, today, 0).library.expect("library");
    assert_eq!(found.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["bench_press"]);
    assert_eq!(found.rows[0].missing.as_deref(), Some("Falta: barra y discos"));

    let equipment = build_view(&data, &catalog, &GymQuery { screen: Screen::Equipo, ..GymQuery::default() }, today, 0).equipment.expect("equipment");
    assert!(equipment.presets.iter().any(|preset| preset.id == "casa" && preset.pressed));
    assert_eq!(equipment.available, "3 de 4");

    let detail = exercise_detail(catalog.exercise("bench_press").expect("exercise"), &catalog, &data);
    assert_eq!(detail.status_text, "Te falta: barra y discos.");
    assert_eq!(detail.muscles.get("upperChest"), Some(&2));
    assert_eq!(detail.projections[0].value, "≈ 60 kcal");
}

#[test]
fn the_exercise_card_edits_cycle_muscles_and_validate() {
    let catalog = catalog();
    let mut exercise = catalog.exercise("dumbbell_curl").expect("exercise").clone();
    apply_exercise_edit(&mut exercise, &ExerciseEdit::CycleMuscle { muscle: "Bíceps".into() }, &catalog).expect("cycle");
    assert_eq!(exercise.muscle_level("biceps"), 1);
    apply_exercise_edit(&mut exercise, &ExerciseEdit::CycleMuscle { muscle: "biceps".into() }, &catalog).expect("cycle");
    assert_eq!(exercise.muscle_level("biceps"), 0);
    apply_exercise_edit(&mut exercise, &ExerciseEdit::Weighted { value: false }, &catalog).expect("weighted");
    assert_eq!(exercise.tracking, Tracking::Reps);
    apply_exercise_edit(&mut exercise, &ExerciseEdit::Kcal { value: "4,5".into() }, &catalog).expect("kcal");
    assert_eq!(exercise.kcal_per_min, 4.5);
    assert!(apply_exercise_edit(&mut exercise, &ExerciseEdit::Kcal { value: "90".into() }, &catalog).is_err());
    assert!(apply_exercise_edit(&mut exercise, &ExerciseEdit::ToggleEquipment { equipment_id: "nope".into() }, &catalog).is_err());
    let custom = saved_equipment(None, "Kettlebell 16 kg", "libres", &catalog, |name| format!("Gym/equipment/{name}.md")).expect("custom");
    assert_eq!((custom.id.as_str(), custom.custom), ("propio_kettlebell_16_kg", true));
    assert!(saved_equipment(None, "mancuernas", "libres", &catalog, |name| name.to_string()).is_err(), "duplicate name");
}

/// Lee los archivos que dejó el importador: `GYM_DIR=…/Gym cargo test -- --ignored`.
#[test]
#[ignore]
fn the_imported_catalog_parses() {
    let Ok(root) = std::env::var("GYM_DIR") else { return };
    let root = std::path::Path::new(&root);
    let read_dir = |folder: &str| {
        let mut files = std::fs::read_dir(root.join(folder))
            .expect("folder")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect::<Vec<_>>();
        files.sort();
        files
    };
    let mut exercises = 0;
    let mut ids = BTreeSet::new();
    for path in read_dir("exercises") {
        let text = std::fs::read_to_string(&path).expect("text");
        let exercise = parse_exercise(&path.to_string_lossy(), &text, true).unwrap_or_else(|| panic!("{}", path.display()));
        assert!(exercise.has_image, "{}", path.display());
        assert!(ids.insert(exercise.id.clone()), "duplicate id {}", exercise.id);
        if let Some(video) = &exercise.video {
            assert!(path.with_file_name(video).exists(), "{video}");
        }
        assert_eq!(parse_exercise("x", &render_exercise(&exercise), true).map(|parsed| parsed.primary), Some(exercise.primary.clone()));
        exercises += 1;
    }
    let mut equipment = 0;
    for path in read_dir("equipment") {
        let text = std::fs::read_to_string(&path).expect("text");
        let item = parse_equipment(&path.to_string_lossy(), &text, false).unwrap_or_else(|| panic!("{}", path.display()));
        assert!(item.has_image, "{}", path.display());
        equipment += 1;
    }
    for sex in [BodySex::Male, BodySex::Female] {
        for back in [false, true] {
            let svg = std::fs::read_to_string(root.join(body::body_file(sex, back))).expect("svg");
            let figure = parse_body_svg(&svg).expect("figure");
            assert!(figure.muscles.len() >= 26, "{} {back}: {}", sex.id(), figure.muscles.len());
        }
    }
    println!("PROBE exercises {exercises} equipment {equipment}");
    assert_eq!((exercises, equipment), (610, 97));
}

#[test]
fn the_tools_resolve_names_and_change_routines_sessions_and_workouts() {
    use super::tools::{action_summary, read_tool, tool_action, GymToolAction};
    use serde_json::json;
    let catalog = catalog();
    let mut data = GymData::default();
    let today_text = "2026-10-01";
    let today = parse_date(today_text).expect("date");
    let run = |data: &mut GymData, name: &str, arguments: serde_json::Value| -> String {
        let action = tool_action(name, &arguments, data, &catalog, today).expect(name);
        let summary = action_summary(&action, data, &catalog);
        let GymToolAction::Change(mutation) = action else { panic!("{name} is a catalog action") };
        apply(data, &catalog, mutation, 1_000_000, today_text);
        summary
    };

    // Una rutina entera, con nombres de ejercicio y días en palabras.
    let summary = run(
        &mut data,
        "save_gym_routine",
        json!({
            "name": "Pecho en casa",
            "days": ["lunes", "jue"],
            "exercises": [
                { "exercise": "press de banca con barra", "sets": [{ "weight": 60, "reps": 8 }, { "weight": 62.5, "reps": 6 }], "restSeconds": 120 },
                { "exercise": "Flexiones de brazos", "sets": 3, "reps": 15 },
                "plank"
            ]
        }),
    );
    assert!(summary.starts_with("Crear la rutina «Pecho en casa»: nombre «Pecho en casa»; días lunes, jueves; 3 ejercicios"), "{summary}");
    let routine = data.routines[0].clone();
    assert_eq!(routine.days, [0, 3]);
    assert_eq!(routine.items[0].sets, [SetPlan { weight: Some(60.0), reps: Some(8.0) }, SetPlan { weight: Some(62.5), reps: Some(6.0) }]);
    assert_eq!((routine.items[0].rest_s, routine.items[1].sets.len()), (120, 3));
    assert_eq!(routine.items[2].sets[0].reps, Some(30.0), "a timed exercise starts with 30 s");

    // Cambiar la lista conserva los ids de los que siguen.
    run(&mut data, "save_gym_routine", json!({ "routine": "pecho en casa", "exercises": ["plank", "bench_press"] }));
    let edited = &data.routines[0];
    assert_eq!(edited.items.iter().map(|item| item.exercise.as_str()).collect::<Vec<_>>(), ["plank", "bench_press"]);
    assert_eq!(edited.items[1].id, routine.items[0].id);
    assert_eq!(edited.items[1].sets, routine.items[0].sets, "what is not given is kept");

    // Entrenar desde el chat.
    run(&mut data, "control_gym_session", json!({ "action": "start", "routine": "Pecho en casa" }));
    run(&mut data, "control_gym_session", json!({ "action": "mark_set", "exercise": "Press de banca con barra", "set": 2 }));
    assert!(data.session.as_ref().is_some_and(|session| session.is_done(&routine.items[0].id, 1)));
    let again = tool_action("control_gym_session", &json!({ "action": "mark_set", "exercise": "bench_press", "set": 2 }), &data, &catalog, today);
    assert!(again.is_err(), "a done set is not marked twice");
    run(&mut data, "control_gym_session", json!({ "action": "finish" }));
    assert_eq!(data.workouts.len(), 1);

    // Un entrenamiento pasado y su borrado.
    let logged = run(&mut data, "log_gym_workout", json!({ "date": "2026-09-29", "minutes": 40, "exercises": [{ "exercise": "Curl con mancuernas", "sets": 3, "reps": 12, "weight": 14 }] }));
    assert!(logged.contains("del 2026-09-29, 40 min: Curl con mancuernas (3 × 12 reps con 14 kg)"), "{logged}");
    let workout = data.workouts.iter().find(|workout| workout.date == "2026-09-29").expect("workout").clone();
    assert_eq!((workout.sets, workout.volume, workout.routine_name.as_str()), (3, 504.0, "Entrenamiento libre"));
    let future = tool_action("log_gym_workout", &json!({ "date": "2026-10-09", "minutes": 40, "routine": "Pecho en casa" }), &data, &catalog, today).expect("arguments");
    let GymToolAction::Change(future) = future else { panic!("a change") };
    let mut new_id = || "x".to_string();
    let refused = plan_change(&data, &catalog, &future, &mut Clock { now_ms: 0, today: today_text.into(), new_id: &mut new_id }).expect_err("future");
    assert_eq!(refused.message, "La fecha no puede ser futura.");
    run(&mut data, "delete_gym_workout", json!({ "workout": "2026-09-29" }));
    assert_eq!(data.workouts.len(), 1);

    // Equipamiento y lecturas.
    run(&mut data, "set_gym_equipment", json!({ "add": ["Mancuernas", "barbell"] }));
    assert!(data.owned.contains("dumbbell") && data.owned.contains("barbell"));
    let found = read_tool("search_gym_exercises", &json!({ "muscle": "pecho inferior", "onlyAvailable": true }), &data, &catalog, today, 0).expect("search");
    assert_eq!(found["total"], 1, "the bench press needs a flat bench too");
    let summary = read_tool("get_gym_summary", &json!({}), &data, &catalog, today, 2_000_000).expect("summary");
    assert_eq!(summary["routines"][0]["days"], json!(["lunes", "jueves"]));
    let detail = read_tool("get_gym_routine", &json!({ "routine": "Pecho en casa" }), &data, &catalog, today, 0).expect("routine");
    assert_eq!(detail["exercises"][1]["sets"][1]["done"], json!(true));

    // Catálogo: crear con nombre y validar referencias.
    let created = tool_action("save_gym_exercise", &json!({ "name": "Remo con banda", "group": "espalda", "primaryMuscles": ["Dorsales"], "equipment": [] }), &data, &catalog, today).expect("create");
    assert!(matches!(created, GymToolAction::SaveExercise { exercise_id: None, .. }));
    assert!(tool_action("save_gym_exercise", &json!({ "name": "Plancha" }), &data, &catalog, today).is_err(), "an existing name is edited, not duplicated");
    let missing = tool_action("get_gym_exercise", &json!({ "exercise": "press" }), &data, &catalog, today).err();
    assert!(missing.is_some());
    let unknown = read_tool("get_gym_exercise", &json!({ "exercise": "press" }), &data, &catalog, today, 0).expect_err("ambiguous");
    assert!(unknown.message.contains("Press de banca con barra"), "{}", unknown.message);
}

#[test]
fn a_routine_change_shows_only_what_changes_and_the_search_reads_every_word() {
    use super::tools::{action_summary, read_tool, tool_action, GymToolAction};
    use serde_json::json;
    let catalog = catalog();
    let mut data = GymData::default();
    let today_text = "2026-10-01";
    let today = parse_date(today_text).expect("date");
    let save = |data: &mut GymData, arguments: serde_json::Value| -> String {
        let action = tool_action("save_gym_routine", &arguments, data, &catalog, today).expect("routine");
        let summary = action_summary(&action, data, &catalog);
        let GymToolAction::Change(mutation) = action else { panic!("a change") };
        apply(data, &catalog, mutation, 1_000_000, today_text);
        summary
    };
    save(
        &mut data,
        json!({ "name": "Piernas", "exercises": [
            { "exercise": "bench_press", "sets": 3, "reps": 7, "weight": 100 },
            { "exercise": "dumbbell_curl", "sets": 3, "reps": 10, "weight": 20 },
            { "exercise": "plank", "sets": 3, "reps": 30 }
        ] }),
    );
    // The whole list comes back with one exercise swapped: only the swap shows.
    let summary = save(
        &mut data,
        json!({ "routine": "Piernas", "exercises": [
            { "exercise": "push_ups", "sets": 3, "reps": 10 },
            { "exercise": "dumbbell_curl", "sets": 3, "reps": 10, "weight": 20 },
            { "exercise": "plank", "sets": 3, "reps": 30 }
        ] }),
    );
    assert_eq!(
        summary,
        "Cambiar la rutina «Piernas»: reemplazar «Press de banca con barra» (3 × 7 reps con 100 kg) por «Flexiones de brazos» (3 × 10 reps)."
    );
    let summary = save(
        &mut data,
        json!({ "routine": "Piernas", "exercises": [
            { "exercise": "plank", "sets": 3, "reps": 30 },
            { "exercise": "dumbbell_curl", "sets": [{ "reps": 10, "weight": 20 }, { "reps": 8, "weight": 22.5 }], "restSeconds": 45 }
        ] }),
    );
    assert_eq!(
        summary,
        "Cambiar la rutina «Piernas»: quitar «Flexiones de brazos» (3 × 10 reps); «Curl con mancuernas»: 3 × 10 reps con 20 kg → 10 reps con 20 kg, 8 reps con 22,5 kg; descanso de «Curl con mancuernas»: 90 s → 45 s; nuevo orden: Plancha, Curl con mancuernas."
    );
    let summary = save(&mut data, json!({ "routine": "Piernas", "exercises": ["plank", "dumbbell_curl"] }));
    assert_eq!(summary, "Cambiar la rutina «Piernas»: los ejercicios quedan igual.");

    // Each word is searched apart, in the name, the muscles and the equipment.
    let search = |query: &str| read_tool("search_gym_exercises", &json!({ "query": query }), &data, &catalog, today, 0).expect("search");
    let found = search("mancuernas bíceps");
    assert_eq!(found["exercises"][0]["name"], "Curl con mancuernas");
    assert!(found.get("partialMatch").is_none());
    let found = search("curl plancha");
    assert_eq!(found["partialMatch"], true);
    assert_eq!(found["total"], 2, "{found}");
}

#[test]
fn ids_helpers() {
    assert_eq!(slug_id("Kettlebell 16 kg"), "kettlebell_16_kg");
    assert_eq!(file_stem("Press: banca / inclinado?"), "Press banca inclinado");
    assert_eq!(rest_clock(90), "1:30");
    assert_eq!(hours_minutes(125.0), "2 h 5 min");
}

/// Vistas de muestra con el catálogo importado, para revisar la pantalla:
/// `GYM_DIR=…/Gym GYM_OUT=… cargo test dump_preview -- --ignored`.
#[test]
#[ignore]
fn dump_preview_fixtures() {
    let (Ok(root), Ok(out)) = (std::env::var("GYM_DIR"), std::env::var("GYM_OUT")) else { return };
    let root = std::path::Path::new(&root);
    let out = std::path::Path::new(&out);
    let mut catalog = Catalog::default();
    for (folder, exercise) in [("exercises", true), ("equipment", false)] {
        for entry in std::fs::read_dir(root.join(folder)).expect("folder").filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("text");
            let logical = format!("Gym/{folder}/{}", path.file_name().unwrap().to_string_lossy());
            if exercise {
                catalog.exercises.extend(parse_exercise(&logical, &text, false));
            } else {
                catalog.equipment.extend(parse_equipment(&logical, &text, false));
            }
        }
    }
    catalog.exercises.sort_by_cached_key(|exercise| fold(&exercise.name));
    let today_text = "2026-10-01";
    let today = parse_date(today_text).expect("date");
    let now = 1_790_895_600_000; // 2026-10-01 19:00 -03
    let mut data = GymData { sex: BodySex::Male, sex_from_profile: true, ..GymData::default() };
    apply(&mut data, &catalog, GymMutation::ApplyPreset { preset: "gym".into() }, now, today_text);
    for (name, focus, day, exercises) in [
        ("Rutina lunes", "Pecho y tríceps", 0u8, ["bench_press", "incline_dumbbell_bench_press", "cable_fly", "dips", "lying_tricep_extension"]),
        ("Rutina miércoles", "Espalda y bíceps", 2, ["pull_ups", "bent_over_row", "lat_pulldown", "barbell_curl", "hammer_curl"]),
        ("Rutina viernes", "Piernas y hombros", 4, ["squat", "sled_leg_press", "romanian_deadlift", "military_press", "dumbbell_lateral_raise"]),
    ] {
        let id = apply(&mut data, &catalog, GymMutation::CreateRoutine, now, today_text).expect("routine");
        apply(&mut data, &catalog, GymMutation::RenameRoutine { routine_id: id.clone(), name: name.into() }, now, today_text);
        apply(&mut data, &catalog, GymMutation::SetFocus { routine_id: id.clone(), focus: focus.into() }, now, today_text);
        apply(&mut data, &catalog, GymMutation::ToggleDay { routine_id: id.clone(), day }, now, today_text);
        for exercise in exercises {
            apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: id.clone(), exercise_id: exercise.into() }, now, today_text);
        }
        let first = data.routine(&id).expect("routine").items[0].id.clone();
        for index in 0..3 {
            apply(&mut data, &catalog, GymMutation::SetValue { routine_id: id.clone(), item_id: first.clone(), index, field: SetField::Weight, value: "60".into() }, now, today_text);
        }
    }
    // Historial: tres días por semana durante 20 semanas.
    let routines = data.routines.clone();
    let monday = parse_date("2026-09-28").expect("date");
    for week in 1..20i64 {
        for (offset, routine) in [(0i64, &routines[0]), (2, &routines[1]), (4, &routines[2])] {
            if (week * 7 + offset) % 9 == 0 {
                continue;
            }
            let date = crate::health::shift_days(monday, -7 * week + offset);
            let minutes = 44.0 + ((week * 3 + offset) % 20) as f64;
            data.workouts.push(Workout {
                id: format!("w-{week}-{offset}"),
                date: date.to_string(),
                routine_id: routine.id.clone(),
                routine_name: routine.name.clone(),
                color: routine.color.clone(),
                ended_ms: now - (7 * week - offset) * 86_400_000,
                minutes,
                kcal: (minutes * 6.0).round(),
                sets: 15,
                volume: 2400.0,
                exercises: routine.items.iter().map(|item| WorkoutExercise { exercise: item.exercise.clone(), sets: item.sets.clone() }).collect(),
                watch_id: None,
                avg_bpm: None,
                max_bpm: None,
            });
        }
    }
    data.workouts.push(Workout {
        id: "w-this-monday".into(),
        date: "2026-09-28".into(),
        routine_id: routines[0].id.clone(),
        routine_name: routines[0].name.clone(),
        color: routines[0].color.clone(),
        ended_ms: now - 3 * 86_400_000 + 3_600_000,
        minutes: 52.0,
        kcal: 312.0,
        sets: 15,
        volume: 2400.0,
        exercises: routines[0].items.iter().map(|item| WorkoutExercise { exercise: item.exercise.clone(), sets: item.sets.clone() }).collect(),
        watch_id: None,
        avg_bpm: None,
        max_bpm: None,
    });
    // Un entrenamiento en curso de la rutina del miércoles.
    let wednesday = &routines[1];
    for index in 0..2 {
        apply(&mut data, &catalog, GymMutation::ToggleSetDone { routine_id: wednesday.id.clone(), item_id: wednesday.items[0].id.clone(), index }, now - 400_000 + index as i64 * 200_000, today_text);
    }
    let write = |name: &str, value: &dyn erased::Json| {
        std::fs::write(out.join(name), value.json()).expect("write");
    };
    for (name, screen, routine) in [
        ("panel", Screen::Panel, None),
        ("rutinas", Screen::Rutinas, Some(routines[0].id.clone())),
        ("editar", Screen::Editar, Some(routines[0].id.clone())),
        ("entrenar", Screen::Entrenar, Some(wednesday.id.clone())),
        ("equipo", Screen::Equipo, None),
    ] {
        let query = GymQuery { screen, routine_id: routine, ..GymQuery::default() };
        write(&format!("view-{name}.json"), &build_view(&data, &catalog, &query, today, now));
    }
    let bench_text = std::fs::read_to_string(root.join("exercises/Press de banca con barra.md")).expect("bench");
    let bench = parse_exercise("Gym/exercises/Press de banca con barra.md", &bench_text, true).expect("bench");
    write("exercise.json", &exercise_detail(&bench, &catalog, &data));
    let figure = |file: &str| parse_body_svg(&std::fs::read_to_string(root.join(file)).expect("svg"));
    write(
        "body.json",
        &body::BodyView { sex: "masculino".into(), front: figure(body::body_file(BodySex::Male, false)), back: figure(body::body_file(BodySex::Male, true)) },
    );
    let mut images = std::collections::BTreeMap::new();
    for item in catalog.equipment.iter().take(40) {
        let text = std::fs::read_to_string(root.join("equipment").join(item.path.rsplit('/').next().unwrap())).expect("equipment");
        if let Some(image) = parse_equipment(&item.path, &text, true).and_then(|item| item.image) {
            images.insert(item.id.clone(), image);
        }
    }
    write("equipment-images.json", &images);
}

mod erased {
    pub trait Json {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> String {
            serde_json::to_string(self).expect("json")
        }
    }
}


/// The watch's sessions complete the Notia workout they overlap (calories
/// and pulse, keeping the sets and the minutes), add the rest as their own
/// workouts, ignore walks, short or odd ones, and update instead of
/// duplicating when imported again. A high sleeping pulse asks for a light
/// day on the panel.
#[test]
fn the_watch_sessions_complete_or_add_workouts() {
    let catalog = catalog();
    let mut data = GymData::default();
    let routine = apply(&mut data, &catalog, GymMutation::CreateRoutine, 0, "2026-10-01").expect("routine");
    apply(&mut data, &catalog, GymMutation::AddExercise { routine_id: routine.clone(), exercise_id: "bench_press".into() }, 0, "2026-10-01");
    // A 50-minute workout finished at 19:00 UTC on 1 Oct 2026.
    let ended = 1_790_881_200_000;
    apply(&mut data, &catalog, GymMutation::LogWorkout { routine_id: Some(routine.clone()), date: "2026-10-01".into(), minutes: 50.0, exercises: vec![WorkoutExercise { exercise: "bench_press".into(), sets: vec![SetPlan { weight: Some(60.0), reps: Some(8.0) }; 3] }] }, ended, "2026-10-01");
    let notia = data.workouts[0].clone();
    let session = |id: &str, kind: &str, date: &str, start: i64, end: i64| WatchSession { id: id.into(), kind: kind.into(), date: date.into(), start_ms: start, end_ms: end, kcal: Some(310.4), avg_bpm: Some(118.2), max_bpm: Some(151.0) };
    let minute = 60_000;
    let sessions = vec![
        session("hc-1", "strength_training", "2026-10-01", ended - 48 * minute, ended + 2 * minute),
        session("hc-2", "running", "2026-09-30", ended - 1_440 * minute, ended - 1_410 * minute),
        session("hc-3", "walking", "2026-09-30", ended - 1_300 * minute, ended - 1_250 * minute),
        session("hc-4", "running", "2026-09-30", ended - 1_200 * minute, ended - 1_197 * minute),
        session("hc-2", "running", "2026-09-30", ended - 1_440 * minute, ended - 1_410 * minute),
        session("hc-5", "biking", "2026-10-09", ended, ended + 40 * minute),
    ];
    apply(&mut data, &catalog, GymMutation::ImportWatchSessions { sessions: sessions.clone() }, ended, "2026-10-01");
    assert_eq!(data.workouts.len(), 2, "{:?}", data.workouts);
    let merged = data.workouts.iter().find(|workout| workout.id == notia.id).expect("merged");
    assert_eq!((merged.watch_id.as_deref(), merged.kcal, merged.avg_bpm, merged.max_bpm), (Some("hc-1"), 310.0, Some(118.0), Some(151.0)));
    assert_eq!((merged.minutes, merged.sets, &merged.exercises), (notia.minutes, notia.sets, &notia.exercises));
    let run = data.workouts.iter().find(|workout| workout.watch_id.as_deref() == Some("hc-2")).expect("run");
    assert_eq!((run.routine_name.as_str(), run.routine_id.as_str(), run.minutes, run.date.as_str()), ("Correr", "", 30.0, "2026-09-30"));
    // Again, with other calories: updated, not duplicated.
    let mut again = sessions;
    again[1].kcal = Some(280.0);
    apply(&mut data, &catalog, GymMutation::ImportWatchSessions { sessions: again }, ended, "2026-10-01");
    assert_eq!(data.workouts.len(), 2);
    assert_eq!(data.workouts.iter().find(|workout| workout.watch_id.as_deref() == Some("hc-2")).expect("run").kcal, 280.0);

    // The sleeping pulse 6 above the two-week average.
    for back in 1..=6 {
        data.sleep_bpm.insert(crate::health::days_ago(parse_date("2026-10-01").expect("date"), back).to_string(), 50.0);
    }
    data.sleep_bpm.insert("2026-10-01".into(), 56.0);
    let panel = build_view(&data, &catalog, &GymQuery::default(), parse_date("2026-10-01").expect("date"), ended).panel.expect("panel");
    assert!(panel.advice.starts_with("Tu pulso dormido está 6 lpm arriba de tu promedio"), "{}", panel.advice);
}
