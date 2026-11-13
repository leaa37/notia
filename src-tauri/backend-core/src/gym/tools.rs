//! Herramientas de la IA sobre Gimnasio. Leen los argumentos, los resuelven
//! contra las rutinas y el catálogo (por id o por nombre) y los convierten en
//! los mismos cambios que manda la pantalla; también arman lo que lee el
//! modelo. Cada usuario ve y cambia solo lo suyo; el catálogo es de la
//! biblioteca.

use jiff::civil::Date;
use serde_json::{json, Value};

use crate::text_search;

use super::change::{ItemInput, SetField};
use super::view::{build_view, exercise_detail, missing_equipment, GymQuery};
use super::{
    category_label, fmt, fold, group_label, is_category, is_group, muscle_key, muscle_label, parse_number, Catalog, Exercise, GymData,
    GymError, GymResult, Routine, SessionStatus, SetPlan, WorkoutExercise, MAX_STEPS, MAX_STEP_CHARS, PRESETS, ROUTINE_COLORS,
};
use super::change::GymMutation;

pub const GYM_READ_TOOLS: [&str; 6] =
    ["get_gym_summary", "get_gym_routine", "search_gym_exercises", "get_gym_exercise", "list_gym_equipment", "list_gym_workouts"];
pub const GYM_WRITE_TOOLS: [&str; 10] = [
    "save_gym_routine",
    "delete_gym_routine",
    "set_gym_equipment",
    "control_gym_session",
    "log_gym_workout",
    "delete_gym_workout",
    "save_gym_exercise",
    "delete_gym_exercise",
    "save_gym_equipment",
    "delete_gym_equipment",
];
const DAY_NAMES: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const DEFAULT_LIMIT: usize = 30;
const MAX_LIMIT: usize = 120;

pub fn is_gym_tool(name: &str) -> bool {
    GYM_READ_TOOLS.contains(&name) || is_gym_write_tool(name)
}

pub fn is_gym_write_tool(name: &str) -> bool {
    GYM_WRITE_TOOLS.contains(&name)
}

/// Lo que cambia la ficha de un ejercicio: solo lo que vino.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExercisePatch {
    pub name: Option<String>,
    pub group: Option<String>,
    pub weighted: Option<bool>,
    pub timed: Option<bool>,
    pub kcal_per_min: Option<f64>,
    pub primary: Option<Vec<String>>,
    pub secondary: Option<Vec<String>>,
    pub equipment: Option<Vec<String>>,
    pub steps: Option<Vec<String>>,
    pub aliases: Option<Vec<String>>,
}

/// Qué hace una llamada de escritura.
#[derive(Debug, Clone, PartialEq)]
pub enum GymToolAction {
    Change(GymMutation),
    /// Crea (sin id) o cambia un ejercicio; `photo` es la imagen del mensaje.
    SaveExercise { exercise_id: Option<String>, patch: ExercisePatch, photo: Option<usize> },
    DeleteExercise { exercise_id: String },
    SaveEquipment { equipment_id: Option<String>, name: String, category: String, photo: Option<usize> },
    DeleteEquipment { equipment_id: String },
}

// ---------------------------------------------------------------------------
// Argumentos
// ---------------------------------------------------------------------------

fn invalid(message: impl Into<String>) -> GymError {
    GymError::validation(message)
}

fn text(arguments: &Value, name: &str) -> Option<String> {
    match arguments.get(name)? {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_number(text),
        _ => None,
    }
}

fn optional_number(arguments: &Value, name: &str) -> GymResult<Option<f64>> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => number(value).map(Some).ok_or_else(|| invalid(format!("{name} tiene que ser un número."))),
    }
}

fn optional_bool(arguments: &Value, name: &str) -> Option<bool> {
    arguments.get(name).and_then(Value::as_bool)
}

fn string_list(arguments: &Value, name: &str) -> GymResult<Option<Vec<String>>> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => Ok(Some(
            items.iter().filter_map(|item| match item {
                Value::String(text) => Some(text.trim().to_string()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|item| !item.is_empty())
            .collect(),
        )),
        Some(Value::String(text)) => Ok(Some(text.split(',').map(|item| item.trim().to_string()).filter(|item| !item.is_empty()).collect())),
        Some(_) => Err(invalid(format!("{name} tiene que ser una lista."))),
    }
}

fn limit(arguments: &Value) -> usize {
    arguments.get("limit").and_then(Value::as_u64).map(|limit| (limit as usize).clamp(1, MAX_LIMIT)).unwrap_or(DEFAULT_LIMIT)
}

fn photo(arguments: &Value) -> GymResult<Option<usize>> {
    match arguments.get("photoFromMessage") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|number| *number >= 1)
            .map(|number| Some(number as usize))
            .ok_or_else(|| invalid("photoFromMessage es el número de la imagen del mensaje, desde 1.")),
    }
}

/// «lunes», «Lun», «L» o 0–6.
fn day_index(value: &str) -> Option<u8> {
    let folded = fold(value.trim());
    if let Ok(number) = folded.parse::<u8>() {
        return (number <= 6).then_some(number);
    }
    DAY_NAMES.iter().position(|name| {
        let name = fold(name);
        name == folded || (folded.len() >= 3 && name.starts_with(&folded))
    })
    .map(|index| index as u8)
}

/// Una rutina por id o por nombre.
pub fn resolve_routine<'a>(data: &'a GymData, reference: &str) -> GymResult<&'a Routine> {
    let folded = fold(reference.trim());
    data.routines
        .iter()
        .find(|routine| routine.id == reference.trim())
        .or_else(|| data.routines.iter().find(|routine| fold(&routine.name) == folded))
        .ok_or_else(|| {
            let names = data.routines.iter().map(|routine| format!("«{}»", routine.name)).collect::<Vec<_>>();
            GymError::not_found(if names.is_empty() {
                "No hay rutinas: creá una con save_gym_routine.".to_string()
            } else {
                format!("No encontré la rutina «{reference}». Las rutinas son: {}.", names.join(", "))
            })
        })
}

/// Un ejercicio por id, nombre o alias exactos (sin acentos ni mayúsculas).
pub fn resolve_exercise<'a>(catalog: &'a Catalog, reference: &str) -> GymResult<&'a Exercise> {
    let reference = reference.trim();
    let folded = fold(reference);
    catalog
        .exercise(reference)
        .or_else(|| catalog.exercises.iter().find(|exercise| fold(&exercise.name) == folded))
        .or_else(|| catalog.exercises.iter().find(|exercise| exercise.aliases.iter().any(|alias| fold(alias) == folded)))
        .ok_or_else(|| {
            let close = catalog
                .exercises
                .iter()
                .filter(|exercise| fold(&exercise.name).contains(&folded) || exercise.aliases.iter().any(|alias| fold(alias).contains(&folded)))
                .take(6)
                .map(|exercise| format!("«{}» ({})", exercise.name, exercise.id))
                .collect::<Vec<_>>();
            GymError::not_found(if close.is_empty() {
                format!("No encontré el ejercicio «{reference}»: buscalo con search_gym_exercises.")
            } else {
                format!("No encontré el ejercicio «{reference}». Parecidos: {}.", close.join(", "))
            })
        })
}

fn resolve_equipment_id(catalog: &Catalog, reference: &str) -> GymResult<String> {
    let folded = fold(reference.trim());
    catalog
        .equipment
        .iter()
        .find(|item| item.id == reference.trim() || fold(&item.name) == folded)
        .map(|item| item.id.clone())
        .ok_or_else(|| GymError::not_found(format!("No encontré el equipamiento «{reference}»: mirá list_gym_equipment.")))
}

fn muscle_keys(values: &[String]) -> GymResult<Vec<String>> {
    let mut keys = Vec::new();
    for value in values {
        let key = muscle_key(value).ok_or_else(|| {
            invalid(format!(
                "El músculo «{value}» no existe. Los músculos son: {}.",
                super::MUSCLES.iter().map(|(_, label, _)| *label).collect::<Vec<_>>().join(", ")
            ))
        })?;
        if !keys.iter().any(|existing| existing == key) {
            keys.push(key.to_string());
        }
    }
    Ok(keys)
}

/// Las series de un ejercicio: una lista `[{weight, reps}]` o
/// `sets` + `reps` (+ `weight`) para todas iguales.
fn sets_argument(value: &Value) -> GymResult<Option<Vec<SetPlan>>> {
    if let Some(list) = value.get("sets").and_then(Value::as_array) {
        return list
            .iter()
            .map(|set| {
                Ok(SetPlan {
                    weight: set.get("weight").and_then(number).or_else(|| set.get("weightKg").and_then(number)),
                    reps: set.get("reps").and_then(number).or_else(|| set.get("seconds").and_then(number)),
                })
            })
            .collect::<GymResult<Vec<_>>>()
            .map(Some);
    }
    let count = value.get("sets").and_then(number);
    let reps = value.get("reps").and_then(number).or_else(|| value.get("seconds").and_then(number));
    let weight = value.get("weight").and_then(number).or_else(|| value.get("weightKg").and_then(number));
    match count {
        Some(count) if (1.0..=20.0).contains(&count) => Ok(Some(vec![SetPlan { weight, reps: reps.or(Some(10.0)) }; count as usize])),
        Some(_) => Err(invalid("sets va de 1 a 20.")),
        None if reps.is_some() || weight.is_some() => Ok(Some(vec![SetPlan { weight, reps: reps.or(Some(10.0)) }; 3])),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Escritura
// ---------------------------------------------------------------------------

fn routine_action(arguments: &Value, data: &GymData, catalog: &Catalog) -> GymResult<GymMutation> {
    let routine = text(arguments, "routine").map(|reference| resolve_routine(data, &reference)).transpose()?;
    let name = text(arguments, "name");
    if routine.is_none() && name.is_none() {
        return Err(invalid("Para una rutina nueva pasá name; para cambiar una, routine con su id o nombre."));
    }
    let days = string_list(arguments, "days")?
        .map(|days| {
            days.iter()
                .map(|day| day_index(day).ok_or_else(|| invalid(format!("El día «{day}» no es válido: usá lunes a domingo."))))
                .collect::<GymResult<Vec<_>>>()
        })
        .transpose()?;
    let color = text(arguments, "color").map(|color| fold(&color).replace('á', "a"));
    if color.as_ref().is_some_and(|color| !ROUTINE_COLORS.contains(&color.as_str())) {
        return Err(invalid(format!("color es uno de: {}.", ROUTINE_COLORS.join(", "))));
    }
    let items = match arguments.get("exercises") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => Some(
            list.iter()
                .map(|entry| {
                    let reference = match entry {
                        Value::String(reference) => reference.clone(),
                        other => text(other, "exercise").ok_or_else(|| invalid("Cada ejercicio necesita exercise (id o nombre)."))?,
                    };
                    let exercise = resolve_exercise(catalog, &reference)?;
                    Ok(ItemInput {
                        exercise_id: exercise.id.clone(),
                        rest_s: entry.get("restSeconds").and_then(number).map(|rest| rest.clamp(0.0, 600.0) as u32),
                        sets: sets_argument(entry)?,
                    })
                })
                .collect::<GymResult<Vec<_>>>()?,
        ),
        Some(_) => return Err(invalid("exercises tiene que ser una lista.")),
    };
    Ok(GymMutation::SaveRoutine {
        routine_id: routine.map(|routine| routine.id.clone()),
        name,
        focus: arguments.get("focus").and_then(Value::as_str).map(str::to_string),
        days,
        color,
        items,
    })
}

/// Un ejercicio de una rutina, por el id del ítem o el del ejercicio, o por
/// su nombre.
fn routine_item<'a>(routine: &'a Routine, catalog: &Catalog, reference: &str) -> GymResult<&'a super::RoutineItem> {
    let by_id = routine.items.iter().find(|item| item.id == reference || item.exercise == reference);
    let found = match by_id {
        Some(item) => Some(item),
        None => {
            let exercise = resolve_exercise(catalog, reference)?;
            routine.items.iter().find(|item| item.exercise == exercise.id)
        }
    };
    found.ok_or_else(|| invalid(format!("«{reference}» no está en {}.", routine.name)))
}

fn session_action(arguments: &Value, data: &GymData, catalog: &Catalog) -> GymResult<GymMutation> {
    let action = text(arguments, "action").ok_or_else(|| invalid("Falta action."))?;
    let active = data.session.as_ref().filter(|session| session.active());
    let routine_for = || -> GymResult<&Routine> {
        match text(arguments, "routine") {
            Some(reference) => resolve_routine(data, &reference),
            None => active
                .and_then(|session| data.routine(&session.routine_id))
                .ok_or_else(|| invalid("Decí qué rutina (routine): no hay un entrenamiento en curso.")),
        }
    };
    Ok(match action.as_str() {
        "start" => GymMutation::StartSession { routine_id: routine_for()?.id.clone() },
        "pause" => GymMutation::PauseSession,
        "resume" => GymMutation::ResumeSession,
        "finish" => GymMutation::FinishSession,
        "restart" => GymMutation::RestartSession,
        "skip_rest" => GymMutation::SkipRest,
        "mark_set" | "unmark_set" => {
            let routine = routine_for()?;
            let reference = text(arguments, "exercise").ok_or_else(|| invalid("Decí de qué ejercicio es la serie (exercise)."))?;
            let item = routine_item(routine, catalog, &reference)?;
            let set = arguments.get("set").and_then(Value::as_u64).filter(|set| *set >= 1).ok_or_else(|| invalid("set es el número de serie, desde 1."))? as usize;
            if set > item.sets.len() {
                return Err(invalid(format!("Ese ejercicio tiene {} series.", item.sets.len())));
            }
            let done = data.session.as_ref().filter(|session| session.routine_id == routine.id).is_some_and(|session| session.is_done(&item.id, set - 1));
            if done == (action == "mark_set") {
                return Err(invalid(if done { "Esa serie ya está hecha." } else { "Esa serie no estaba hecha." }));
            }
            GymMutation::ToggleSetDone { routine_id: routine.id.clone(), item_id: item.id.clone(), index: set - 1 }
        }
        "set_value" => {
            let routine = routine_for()?;
            let reference = text(arguments, "exercise").ok_or_else(|| invalid("Decí de qué ejercicio (exercise)."))?;
            let item = routine_item(routine, catalog, &reference)?;
            let set = arguments.get("set").and_then(Value::as_u64).filter(|set| *set >= 1).ok_or_else(|| invalid("set es el número de serie, desde 1."))? as usize;
            let (field, value) = match (arguments.get("weight"), arguments.get("reps")) {
                (Some(weight), _) => (SetField::Weight, number(weight).map(|value| value.to_string()).unwrap_or_default()),
                (_, Some(reps)) => (SetField::Reps, number(reps).map(|value| value.to_string()).unwrap_or_default()),
                _ => return Err(invalid("Pasá weight o reps.")),
            };
            GymMutation::SetValue { routine_id: routine.id.clone(), item_id: item.id.clone(), index: set - 1, field, value }
        }
        _ => return Err(invalid("action es start, pause, resume, finish, restart, mark_set, unmark_set, set_value o skip_rest.")),
    })
}

fn workout_action(arguments: &Value, data: &GymData, catalog: &Catalog, today: Date) -> GymResult<GymMutation> {
    let routine = text(arguments, "routine").map(|reference| resolve_routine(data, &reference)).transpose()?;
    let minutes = optional_number(arguments, "minutes")?.ok_or_else(|| invalid("Falta minutes (cuánto duró)."))?;
    let exercises = match arguments.get("exercises") {
        Some(Value::Array(list)) if !list.is_empty() => list
            .iter()
            .map(|entry| {
                let reference = text(entry, "exercise").ok_or_else(|| invalid("Cada ejercicio necesita exercise (id o nombre)."))?;
                let exercise = resolve_exercise(catalog, &reference)?;
                let sets = sets_argument(entry)?.ok_or_else(|| invalid(format!("Faltan las series de {}.", exercise.name)))?;
                Ok(WorkoutExercise { exercise: exercise.id.clone(), sets })
            })
            .collect::<GymResult<Vec<_>>>()?,
        // Sin ejercicios: los de la rutina, como estaban planificados.
        _ => match routine {
            Some(routine) => routine.items.iter().map(|item| WorkoutExercise { exercise: item.exercise.clone(), sets: item.sets.clone() }).collect(),
            None => return Err(invalid("Pasá exercises con sus series, o routine para usar los de la rutina.")),
        },
    };
    Ok(GymMutation::LogWorkout {
        routine_id: routine.map(|routine| routine.id.clone()),
        date: text(arguments, "date").unwrap_or_else(|| today.to_string()),
        minutes,
        exercises,
    })
}

fn exercise_patch(arguments: &Value, catalog: &Catalog) -> GymResult<ExercisePatch> {
    let group = text(arguments, "group").map(|group| fold(&group));
    if group.as_ref().is_some_and(|group| !is_group(group)) {
        return Err(invalid(format!("group es uno de: {}.", super::GROUPS.iter().map(|(key, _)| *key).collect::<Vec<_>>().join(", "))));
    }
    let kcal = optional_number(arguments, "kcalPerMinute")?;
    if kcal.is_some_and(|kcal| !(0.0..=40.0).contains(&kcal)) {
        return Err(invalid("kcalPerMinute va de 0 a 40."));
    }
    let steps = string_list(arguments, "steps")?;
    if steps.as_ref().is_some_and(|steps| steps.len() > MAX_STEPS || steps.iter().any(|step| step.chars().count() > MAX_STEP_CHARS)) {
        return Err(invalid(format!("Hasta {MAX_STEPS} pasos de hasta {MAX_STEP_CHARS} caracteres.")));
    }
    Ok(ExercisePatch {
        name: text(arguments, "name"),
        group,
        weighted: optional_bool(arguments, "weighted"),
        timed: optional_bool(arguments, "timed"),
        kcal_per_min: kcal,
        primary: string_list(arguments, "primaryMuscles")?.map(|muscles| muscle_keys(&muscles)).transpose()?,
        secondary: string_list(arguments, "secondaryMuscles")?.map(|muscles| muscle_keys(&muscles)).transpose()?,
        equipment: string_list(arguments, "equipment")?
            .map(|items| items.iter().map(|item| resolve_equipment_id(catalog, item)).collect::<GymResult<Vec<_>>>())
            .transpose()?,
        steps,
        aliases: string_list(arguments, "aliases")?,
    })
}

/// Aplica lo que vino en la ficha de un ejercicio.
pub fn apply_patch(exercise: &mut Exercise, patch: &ExercisePatch) -> GymResult<()> {
    if let Some(name) = &patch.name {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > super::MAX_NAME_CHARS {
            return Err(invalid("El nombre tiene de 1 a 120 caracteres."));
        }
        exercise.name = name;
    }
    if let Some(group) = &patch.group {
        exercise.group = group.clone();
    }
    if let Some(weighted) = patch.weighted {
        exercise.tracking = exercise.tracking.with_weight(weighted);
    }
    if let Some(timed) = patch.timed {
        let weighted = exercise.tracking.weighted();
        exercise.tracking = match (timed, weighted) {
            (true, true) => super::Tracking::WeightTime,
            (true, false) => super::Tracking::Time,
            (false, true) => super::Tracking::WeightReps,
            (false, false) => super::Tracking::Reps,
        };
    }
    if let Some(kcal) = patch.kcal_per_min {
        exercise.kcal_per_min = (kcal * 10.0).round() / 10.0;
    }
    if let Some(primary) = &patch.primary {
        exercise.primary = primary.clone();
        exercise.secondary.retain(|muscle| !primary.contains(muscle));
    }
    if let Some(secondary) = &patch.secondary {
        exercise.secondary = secondary.iter().filter(|muscle| !exercise.primary.contains(muscle)).cloned().collect();
    }
    if let Some(equipment) = &patch.equipment {
        exercise.equipment = equipment.clone();
    }
    if let Some(steps) = &patch.steps {
        exercise.steps = steps.clone();
    }
    if let Some(aliases) = &patch.aliases {
        exercise.aliases = aliases.clone();
    }
    Ok(())
}

/// Qué hace una llamada de escritura, validada. No guarda nada.
pub fn tool_action(name: &str, arguments: &Value, data: &GymData, catalog: &Catalog, today: Date) -> GymResult<GymToolAction> {
    Ok(match name {
        "save_gym_routine" => GymToolAction::Change(routine_action(arguments, data, catalog)?),
        "delete_gym_routine" => {
            let reference = text(arguments, "routine").ok_or_else(|| invalid("Falta routine (id o nombre)."))?;
            GymToolAction::Change(GymMutation::DeleteRoutine { routine_id: resolve_routine(data, &reference)?.id.clone() })
        }
        "set_gym_equipment" => {
            if let Some(preset) = text(arguments, "preset") {
                if !PRESETS.iter().any(|(id, _)| *id == preset) {
                    return Err(invalid("preset es gym (gimnasio completo), casa (casa con mancuernas) o nada."));
                }
                return Ok(GymToolAction::Change(GymMutation::ApplyPreset { preset }));
            }
            let ids = |key: &str| -> GymResult<Vec<String>> {
                string_list(arguments, key)?.unwrap_or_default().iter().map(|item| resolve_equipment_id(catalog, item)).collect()
            };
            let (add, remove) = (ids("add")?, ids("remove")?);
            if add.is_empty() && remove.is_empty() {
                return Err(invalid("Pasá preset, o add y remove con el equipamiento."));
            }
            GymToolAction::Change(GymMutation::SetEquipment { add, remove })
        }
        "control_gym_session" => GymToolAction::Change(session_action(arguments, data, catalog)?),
        "log_gym_workout" => GymToolAction::Change(workout_action(arguments, data, catalog, today)?),
        "delete_gym_workout" => {
            let reference = text(arguments, "workout").ok_or_else(|| invalid("Falta workout (su id de list_gym_workouts)."))?;
            let workout = data
                .workouts
                .iter()
                .find(|workout| workout.id == reference)
                .or_else(|| {
                    let on_date = data.workouts.iter().filter(|workout| workout.date == reference).collect::<Vec<_>>();
                    (on_date.len() == 1).then(|| on_date[0])
                })
                .ok_or_else(|| GymError::not_found("No encontré ese entrenamiento: pasá su id de list_gym_workouts."))?;
            GymToolAction::Change(GymMutation::DeleteWorkout { workout_id: workout.id.clone() })
        }
        "save_gym_exercise" => {
            let exercise_id = text(arguments, "exercise").map(|reference| resolve_exercise(catalog, &reference).map(|exercise| exercise.id.clone())).transpose()?;
            let patch = exercise_patch(arguments, catalog)?;
            if exercise_id.is_none() && patch.name.is_none() {
                return Err(invalid("Para un ejercicio nuevo pasá name; para cambiar uno, exercise con su id o nombre."));
            }
            if let (None, Some(name)) = (&exercise_id, &patch.name) {
                if let Ok(existing) = resolve_exercise(catalog, name) {
                    return Err(invalid(format!("Ya existe «{}» ({}): cambialo pasando exercise.", existing.name, existing.id)));
                }
            }
            GymToolAction::SaveExercise { exercise_id, patch, photo: photo(arguments)? }
        }
        "delete_gym_exercise" => {
            let reference = text(arguments, "exercise").ok_or_else(|| invalid("Falta exercise (id o nombre)."))?;
            GymToolAction::DeleteExercise { exercise_id: resolve_exercise(catalog, &reference)?.id.clone() }
        }
        "save_gym_equipment" => {
            let existing = text(arguments, "equipment").map(|reference| resolve_equipment_id(catalog, &reference)).transpose()?;
            let current = existing.as_deref().and_then(|id| catalog.equipment(id));
            let name = text(arguments, "name").or_else(|| current.map(|item| item.name.clone())).ok_or_else(|| invalid("Falta name."))?;
            let category = text(arguments, "category").map(|category| fold(&category)).or_else(|| current.map(|item| item.category.clone())).unwrap_or_else(|| "otros".to_string());
            if !is_category(&category) {
                return Err(invalid("category es libres, estructuras, maquinas, accesorios u otros."));
            }
            GymToolAction::SaveEquipment { equipment_id: existing, name, category, photo: photo(arguments)? }
        }
        "delete_gym_equipment" => {
            let reference = text(arguments, "equipment").ok_or_else(|| invalid("Falta equipment (id o nombre)."))?;
            GymToolAction::DeleteEquipment { equipment_id: resolve_equipment_id(catalog, &reference)? }
        }
        _ => return Err(invalid("La herramienta de Gimnasio no existe.")),
    })
}

/// Las series en pocas palabras: las iguales seguidas van juntas
/// («3 × 10 reps con 20 kg»).
fn sets_text(sets: &[SetPlan], timed: bool) -> String {
    let unit = if timed { "s" } else { "reps" };
    let one = |set: &SetPlan| match set.weight.filter(|weight| *weight > 0.0) {
        Some(weight) => format!("{} {unit} con {} kg", fmt(set.reps.unwrap_or(0.0), 1), fmt(weight, 2)),
        None => format!("{} {unit}", fmt(set.reps.unwrap_or(0.0), 1)),
    };
    let mut runs = Vec::<(usize, &SetPlan)>::new();
    for set in sets {
        match runs.last_mut() {
            Some((count, last)) if *last == set => *count += 1,
            _ => runs.push((1, set)),
        }
    }
    runs.iter()
        .map(|(count, set)| if *count > 1 { format!("{count} × {}", one(set)) } else { one(set) })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Qué cambia en los ejercicios de una rutina que ya existe: solo lo
/// distinto, no la lista entera.
fn routine_items_change(routine: &Routine, items: &[ItemInput], catalog: &Catalog) -> String {
    let name = |id: &str| catalog.exercise(id).map(|exercise| exercise.name.clone()).unwrap_or_else(|| id.to_string());
    let timed = |id: &str| catalog.exercise(id).is_some_and(|exercise| exercise.tracking.timed());
    let described = |id: &str, sets: Option<&[SetPlan]>| match sets {
        Some(sets) if !sets.is_empty() => format!("«{}» ({})", name(id), sets_text(sets, timed(id))),
        _ => format!("«{}»", name(id)),
    };
    // Cada ejercicio nuevo se empareja con el primero igual que no se usó,
    // como al guardar.
    let mut used = vec![false; routine.items.len()];
    let mut added = Vec::new();
    let mut kept = Vec::new();
    for (position, item) in items.iter().enumerate() {
        match routine.items.iter().enumerate().find(|(index, current)| !used[*index] && current.exercise == item.exercise_id) {
            Some((index, current)) => {
                used[index] = true;
                kept.push((index, item, current));
            }
            None => added.push((position, item)),
        }
    }
    let removed = routine.items.iter().enumerate().filter(|(index, _)| !used[*index]).collect::<Vec<_>>();
    let mut parts = Vec::new();
    match (removed.as_slice(), added.as_slice()) {
        ([(_, old)], [(_, new)]) => parts.push(format!(
            "reemplazar {} por {}",
            described(&old.exercise, Some(&old.sets)),
            described(&new.exercise_id, new.sets.as_deref())
        )),
        _ => {
            if !removed.is_empty() {
                parts.push(format!("quitar {}", removed.iter().map(|(_, old)| described(&old.exercise, Some(&old.sets))).collect::<Vec<_>>().join(", ")));
            }
            if !added.is_empty() {
                parts.push(format!("agregar {}", added.iter().map(|(_, new)| described(&new.exercise_id, new.sets.as_deref())).collect::<Vec<_>>().join(", ")));
            }
        }
    }
    for (_, item, current) in &kept {
        if let Some(sets) = item.sets.as_ref().filter(|sets| **sets != current.sets) {
            parts.push(format!("«{}»: {} → {}", name(&current.exercise), sets_text(&current.sets, timed(&current.exercise)), sets_text(sets, timed(&current.exercise))));
        }
        if let Some(rest) = item.rest_s.filter(|rest| *rest != current.rest_s) {
            parts.push(format!("descanso de «{}»: {} s → {rest} s", name(&current.exercise), current.rest_s));
        }
    }
    if kept.windows(2).any(|pair| pair[0].0 > pair[1].0) {
        parts.push(format!("nuevo orden: {}", items.iter().map(|item| name(&item.exercise_id)).collect::<Vec<_>>().join(", ")));
    }
    if parts.is_empty() {
        "los ejercicios quedan igual".to_string()
    } else {
        parts.join("; ")
    }
}

/// Lo que se confirma, en palabras.
pub fn action_summary(action: &GymToolAction, data: &GymData, catalog: &Catalog) -> String {
    let exercise_name = |id: &str| catalog.exercise(id).map(|exercise| exercise.name.clone()).unwrap_or_else(|| id.to_string());
    let routine_name = |id: &str| data.routine(id).map(|routine| routine.name.clone()).unwrap_or_else(|| id.to_string());
    match action {
        GymToolAction::Change(mutation) => match mutation {
            GymMutation::SaveRoutine { routine_id, name, focus, days, color, items } => {
                let mut parts = Vec::new();
                if let Some(name) = name {
                    parts.push(format!("nombre «{name}»"));
                }
                if let Some(focus) = focus {
                    parts.push(format!("enfoque «{focus}»"));
                }
                if let Some(days) = days {
                    parts.push(if days.is_empty() {
                        "sin días".to_string()
                    } else {
                        format!("días {}", days.iter().map(|day| DAY_NAMES[*day as usize]).collect::<Vec<_>>().join(", "))
                    });
                }
                if let Some(color) = color {
                    parts.push(format!("color {color}"));
                }
                if let Some(routine) = items.as_ref().and(routine_id.as_ref()).and_then(|id| data.routine(id)) {
                    parts.push(routine_items_change(routine, items.as_deref().unwrap_or_default(), catalog));
                } else if let Some(items) = items {
                    let list = items
                        .iter()
                        .map(|item| {
                            let timed = catalog.exercise(&item.exercise_id).is_some_and(|exercise| exercise.tracking.timed());
                            match &item.sets {
                                Some(sets) => format!("{} ({})", exercise_name(&item.exercise_id), sets_text(sets, timed)),
                                None => exercise_name(&item.exercise_id),
                            }
                        })
                        .collect::<Vec<_>>();
                    parts.push(format!("{} ejercicios: {}", list.len(), list.join("; ")));
                }
                match routine_id {
                    Some(id) => format!("Cambiar la rutina «{}»: {}.", routine_name(id), parts.join("; ")),
                    None => format!("Crear la rutina «{}»: {}.", name.as_deref().unwrap_or("Nueva rutina"), parts.join("; ")),
                }
            }
            GymMutation::DeleteRoutine { routine_id } => format!("Eliminar la rutina «{}». El historial de entrenamientos se conserva.", routine_name(routine_id)),
            GymMutation::ApplyPreset { preset } => format!(
                "Marcar el equipamiento de «{}».",
                PRESETS.iter().find(|(id, _)| id == preset).map(|(_, label)| *label).unwrap_or(preset.as_str())
            ),
            GymMutation::SetEquipment { add, remove } => {
                let names = |ids: &[String]| ids.iter().filter_map(|id| catalog.equipment_name(id)).collect::<Vec<_>>().join(", ");
                match (add.is_empty(), remove.is_empty()) {
                    (false, false) => format!("Marcar como disponible {} y desmarcar {}.", names(add), names(remove)),
                    (false, true) => format!("Marcar como disponible: {}.", names(add)),
                    _ => format!("Desmarcar: {}.", names(remove)),
                }
            }
            GymMutation::StartSession { routine_id } => format!("Empezar a entrenar «{}».", routine_name(routine_id)),
            GymMutation::PauseSession => "Pausar el entrenamiento en curso.".to_string(),
            GymMutation::ResumeSession => "Reanudar el entrenamiento.".to_string(),
            GymMutation::FinishSession => "Terminar el entrenamiento y guardarlo en el historial.".to_string(),
            GymMutation::RestartSession => "Descartar el entrenamiento en curso y volver a empezar.".to_string(),
            GymMutation::SkipRest => "Saltar el descanso.".to_string(),
            GymMutation::ToggleSetDone { routine_id, item_id, index } => {
                let exercise = data.routine(routine_id).and_then(|routine| routine.items.iter().find(|item| &item.id == item_id)).map(|item| exercise_name(&item.exercise)).unwrap_or_default();
                let done = data.session.as_ref().is_some_and(|session| session.is_done(item_id, *index));
                format!("{} la serie {} de {exercise}.", if done { "Desmarcar" } else { "Marcar como hecha" }, index + 1)
            }
            GymMutation::SetValue { routine_id, item_id, index, field, value } => {
                let exercise = data.routine(routine_id).and_then(|routine| routine.items.iter().find(|item| &item.id == item_id)).map(|item| exercise_name(&item.exercise)).unwrap_or_default();
                format!("Poner {} {value} en la serie {} de {exercise}.", if *field == SetField::Weight { "peso" } else { "repeticiones" }, index + 1)
            }
            GymMutation::LogWorkout { routine_id, date, minutes, exercises } => format!(
                "Registrar un entrenamiento{} del {date}, {} min: {}.",
                routine_id.as_deref().map(|id| format!(" de «{}»", routine_name(id))).unwrap_or_default(),
                fmt(*minutes, 0),
                exercises
                    .iter()
                    .map(|item| {
                        let timed = catalog.exercise(&item.exercise).is_some_and(|exercise| exercise.tracking.timed());
                        format!("{} ({})", exercise_name(&item.exercise), sets_text(&item.sets, timed))
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            GymMutation::DeleteWorkout { workout_id } => {
                let workout = data.workouts.iter().find(|workout| &workout.id == workout_id);
                format!(
                    "Eliminar del historial el entrenamiento {}.",
                    workout.map(|workout| format!("del {} ({}, {} min)", workout.date, workout.routine_name, fmt(workout.minutes, 0))).unwrap_or_default()
                )
            }
            other => format!("Aplicar el cambio de Gimnasio {other:?}."),
        },
        GymToolAction::SaveExercise { exercise_id, patch, photo } => {
            let mut parts = Vec::new();
            if let Some(name) = &patch.name {
                parts.push(format!("nombre «{name}»"));
            }
            if let Some(group) = &patch.group {
                parts.push(format!("grupo {}", group_label(group)));
            }
            if let Some(weighted) = patch.weighted {
                parts.push(if weighted { "con peso".to_string() } else { "con peso corporal".to_string() });
            }
            if let Some(timed) = patch.timed {
                parts.push(if timed { "por tiempo".to_string() } else { "por repeticiones".to_string() });
            }
            if let Some(kcal) = patch.kcal_per_min {
                parts.push(format!("{} kcal por minuto", fmt(kcal, 1)));
            }
            let muscles = |keys: &[String]| keys.iter().filter_map(|key| muscle_label(key)).collect::<Vec<_>>().join(", ");
            if let Some(primary) = &patch.primary {
                parts.push(format!("principales: {}", muscles(primary)));
            }
            if let Some(secondary) = &patch.secondary {
                parts.push(format!("secundarios: {}", muscles(secondary)));
            }
            if let Some(equipment) = &patch.equipment {
                let names = equipment.iter().filter_map(|id| catalog.equipment_name(id)).collect::<Vec<_>>();
                parts.push(if names.is_empty() { "sin equipamiento".to_string() } else { format!("equipamiento: {}", names.join(", ")) });
            }
            if let Some(steps) = &patch.steps {
                parts.push(format!("{} pasos", steps.len()));
            }
            if photo.is_some() {
                parts.push("con la imagen del mensaje".to_string());
            }
            match exercise_id {
                Some(id) => format!("Cambiar la ficha de «{}» en Gym/exercises: {}.", exercise_name(id), parts.join("; ")),
                None => format!("Crear el ejercicio en Gym/exercises: {}.", parts.join("; ")),
            }
        }
        GymToolAction::DeleteExercise { exercise_id } => {
            let routines = data.routines.iter().filter(|routine| routine.items.iter().any(|item| &item.exercise == exercise_id)).map(|routine| format!("«{}»", routine.name)).collect::<Vec<_>>();
            format!(
                "Borrar el ejercicio «{}» de Gym/exercises (y su video).{}",
                exercise_name(exercise_id),
                if routines.is_empty() { String::new() } else { format!(" Queda como «Ejercicio eliminado» en {}.", routines.join(", ")) }
            )
        }
        GymToolAction::SaveEquipment { equipment_id, name, category, photo } => format!(
            "{} «{name}» ({}) en Gym/equipment{}.",
            if equipment_id.is_some() { "Cambiar el equipamiento" } else { "Agregar el equipamiento" },
            category_label(category),
            if photo.is_some() { ", con la foto del mensaje" } else { "" }
        ),
        GymToolAction::DeleteEquipment { equipment_id } => format!("Quitar el equipamiento «{}» de Gym/equipment.", catalog.equipment_name(equipment_id).unwrap_or(equipment_id)),
    }
}

// ---------------------------------------------------------------------------
// Lectura
// ---------------------------------------------------------------------------

fn days_text(days: &[u8]) -> Vec<&'static str> {
    days.iter().filter_map(|day| DAY_NAMES.get(*day as usize)).copied().collect()
}

/// Lo que la búsqueda mira de un ejercicio: nombre, alias, grupo,
/// músculos y equipamiento.
fn exercise_search_text(exercise: &Exercise, catalog: &Catalog) -> String {
    let mut parts = vec![exercise.name.clone(), group_label(&exercise.group).to_string()];
    parts.extend(exercise.aliases.iter().cloned());
    parts.extend(exercise.primary.iter().chain(&exercise.secondary).filter_map(|key| muscle_label(key)).map(str::to_string));
    parts.extend(exercise.equipment.iter().filter_map(|id| catalog.equipment_name(id)).map(str::to_string));
    text_search::fold(&parts.join(" | "))
}

fn exercise_row(exercise: &Exercise, catalog: &Catalog, data: &GymData) -> Value {
    let names = |keys: &[String]| keys.iter().filter_map(|key| muscle_label(key)).collect::<Vec<_>>();
    json!({
        "id": exercise.id,
        "name": exercise.name,
        "group": exercise.group,
        "tracking": exercise.tracking.id(),
        "primaryMuscles": names(&exercise.primary),
        "secondaryMuscles": names(&exercise.secondary),
        "equipment": exercise.equipment.iter().filter_map(|id| catalog.equipment_name(id)).collect::<Vec<_>>(),
        "missingEquipment": missing_equipment(exercise, catalog, &data.owned),
        "kcalPerMinute": exercise.kcal_per_min,
    })
}

fn routine_json(routine: &Routine, catalog: &Catalog, data: &GymData) -> Value {
    let session = data.session.as_ref().filter(|session| session.routine_id == routine.id);
    json!({
        "id": routine.id,
        "name": routine.name,
        "focus": routine.focus,
        "days": days_text(&routine.days),
        "color": routine.color,
        "exercises": routine.items.iter().map(|item| {
            let exercise = catalog.exercise(&item.exercise);
            json!({
                "itemId": item.id,
                "exercise": item.exercise,
                "name": exercise.map(|exercise| exercise.name.as_str()).unwrap_or("Ejercicio eliminado"),
                "tracking": exercise.map(|exercise| exercise.tracking.id()),
                "restSeconds": item.rest_s,
                "sets": item.sets.iter().enumerate().map(|(index, set)| json!({
                    "set": index + 1,
                    "weight": set.weight,
                    "reps": set.reps,
                    "done": session.map(|session| session.is_done(&item.id, index)),
                })).collect::<Vec<_>>(),
                "missingEquipment": exercise.map(|exercise| missing_equipment(exercise, catalog, &data.owned)).unwrap_or_default(),
            })
        }).collect::<Vec<_>>(),
    })
}

/// Lo que lee el modelo con una herramienta de lectura.
pub fn read_tool(name: &str, arguments: &Value, data: &GymData, catalog: &Catalog, today: Date, now_ms: i64) -> GymResult<Value> {
    Ok(match name {
        "get_gym_summary" => {
            let view = build_view(data, catalog, &GymQuery::default(), today, now_ms);
            let panel = view.panel.expect("panel");
            let session = data.session.as_ref().filter(|session| session.active());
            json!({
                "today": today.to_string(),
                "bodySex": data.sex.id(),
                "routines": data.routines.iter().map(|routine| json!({
                    "id": routine.id,
                    "name": routine.name,
                    "focus": routine.focus,
                    "days": days_text(&routine.days),
                    "exercises": routine.items.len(),
                    "sets": routine.items.iter().map(|item| item.sets.len()).sum::<usize>(),
                })).collect::<Vec<_>>(),
                "week": panel.week_label,
                "next": panel.next_label,
                "stats": panel.stats.iter().map(|stat| json!({ "key": stat.key, "value": stat.value, "detail": stat.sub })).collect::<Vec<_>>(),
                "muscles": panel.muscles.iter().filter(|muscle| muscle.state != "none").map(|muscle| json!({ "muscle": muscle.name, "state": muscle.detail })).collect::<Vec<_>>(),
                "advice": panel.advice,
                "session": session.map(|session| {
                    let routine = data.routine(&session.routine_id);
                    let total: usize = routine.map(|routine| routine.items.iter().map(|item| item.sets.len()).sum()).unwrap_or(0);
                    let done: usize = session.done.values().map(|sets| sets.iter().filter(|done| **done).count()).sum();
                    json!({
                        "routine": routine.map(|routine| routine.name.clone()),
                        "status": if session.status == SessionStatus::Paused { "paused" } else { "running" },
                        "minutes": (session.elapsed_ms(now_ms) as f64 / 60000.0).round(),
                        "setsDone": done,
                        "setsTotal": total,
                    })
                }),
                "equipment": { "owned": view.equipment_owned, "total": view.equipment_total },
                "exerciseCount": view.exercise_total,
                "lastWorkouts": data.workouts.iter().rev().take(5).map(|workout| json!({
                    "id": workout.id, "date": workout.date, "routine": workout.routine_name, "minutes": workout.minutes, "kcal": workout.kcal, "sets": workout.sets,
                })).collect::<Vec<_>>(),
            })
        }
        "get_gym_routine" => {
            let reference = text(arguments, "routine").ok_or_else(|| invalid("Falta routine (id o nombre)."))?;
            routine_json(resolve_routine(data, &reference)?, catalog, data)
        }
        "search_gym_exercises" => {
            let terms = text_search::query_terms(&text(arguments, "query").unwrap_or_default());
            let group = text(arguments, "group").map(|group| fold(&group));
            let muscle = text(arguments, "muscle").map(|muscle| muscle_key(&muscle).ok_or_else(|| invalid(format!("El músculo «{muscle}» no existe.")))).transpose()?;
            let only_available = optional_bool(arguments, "onlyAvailable").unwrap_or(false);
            let candidates = catalog
                .exercises
                .iter()
                .filter(|exercise| group.as_ref().is_none_or(|group| &exercise.group == group))
                .filter(|exercise| muscle.is_none_or(|muscle| exercise.muscle_level(muscle) > 0))
                .filter(|exercise| !only_available || missing_equipment(exercise, catalog, &data.owned).is_empty());
            let found = text_search::rank(candidates, &terms, |exercise| exercise_search_text(exercise, catalog));
            let mut result = json!({
                "total": found.items.len(),
                "exercises": found.items.iter().take(limit(arguments)).map(|exercise| exercise_row(exercise, catalog, data)).collect::<Vec<_>>(),
            });
            if found.partial {
                result["partialMatch"] = json!(true);
                result["note"] = json!("Ningún ejercicio tiene todas las palabras; estos tienen alguna, los que más primero.");
            }
            result
        }
        "get_gym_exercise" => {
            let reference = text(arguments, "exercise").ok_or_else(|| invalid("Falta exercise (id o nombre)."))?;
            let exercise = resolve_exercise(catalog, &reference)?;
            let detail = exercise_detail(exercise, catalog, data);
            json!({
                "id": detail.id,
                "name": detail.name,
                "group": detail.group_label,
                "tracking": detail.tracking,
                "kcalPerMinute": exercise.kcal_per_min,
                "primaryMuscles": detail.primary_text,
                "secondaryMuscles": detail.secondary_text,
                "equipment": detail.required_text,
                "equipmentStatus": detail.status_text,
                "steps": detail.steps,
                "hasImage": exercise.has_image,
                "hasVideo": detail.has_video,
                "aliases": exercise.aliases,
                "file": detail.path,
                "inRoutines": data.routines.iter().filter(|routine| routine.items.iter().any(|item| item.exercise == exercise.id)).map(|routine| routine.name.clone()).collect::<Vec<_>>(),
            })
        }
        "list_gym_equipment" => json!({
            "equipment": catalog.equipment.iter().map(|item| json!({
                "id": item.id,
                "name": item.name,
                "category": category_label(&item.category),
                "owned": data.owned.contains(&item.id),
                "custom": item.custom,
                "usedBy": catalog.exercises.iter().filter(|exercise| exercise.equipment.contains(&item.id)).count(),
            })).collect::<Vec<_>>(),
        }),
        "list_gym_workouts" => {
            let from = text(arguments, "from").and_then(|date| super::parse_date(&date));
            let to = text(arguments, "to").and_then(|date| super::parse_date(&date));
            let workouts = data
                .workouts
                .iter()
                .rev()
                .filter(|workout| {
                    let date = super::parse_date(&workout.date);
                    from.is_none_or(|from| date.is_some_and(|date| date >= from)) && to.is_none_or(|to| date.is_some_and(|date| date <= to))
                })
                .collect::<Vec<_>>();
            json!({
                "total": workouts.len(),
                "workouts": workouts.iter().take(limit(arguments)).map(|workout| json!({
                    "id": workout.id,
                    "date": workout.date,
                    "routine": workout.routine_name,
                    "minutes": workout.minutes,
                    "kcal": workout.kcal,
                    "sets": workout.sets,
                    "volumeKg": workout.volume,
                    "fromWatch": workout.watch_id.is_some(),
                    "avgBpm": workout.avg_bpm,
                    "maxBpm": workout.max_bpm,
                    "exercises": workout.exercises.iter().map(|item| json!({
                        "exercise": catalog.exercise(&item.exercise).map(|exercise| exercise.name.clone()).unwrap_or_else(|| item.exercise.clone()),
                        "sets": item.sets,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        }
        _ => return Err(invalid("La herramienta de Gimnasio no existe.")),
    })
}

/// Los grupos y músculos válidos, para los mensajes y la guía del modelo.
pub fn vocabulary() -> String {
    format!(
        "Grupos: {}. Músculos: {}.",
        super::GROUPS.iter().map(|(key, _)| *key).collect::<Vec<_>>().join(", "),
        super::MUSCLES.iter().map(|(_, label, _)| *label).collect::<Vec<_>>().join(", ")
    )
}
