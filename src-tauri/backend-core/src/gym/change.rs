//! Los cambios de la pantalla sobre los datos del usuario: rutinas,
//! equipamiento, sesión de entrenamiento e historial. Cada cambio se valida
//! acá y se traduce en operaciones que el adaptador guarda en una
//! transacción.

use serde::Deserialize;

use super::{
    Catalog, GymData, GymError, GymResult, Routine, RoutineItem, Session, SessionStatus, SetPlan, Workout, WorkoutExercise,
    DEFAULT_REST_S, HOME_PRESET, MAX_NAME_CHARS, MAX_REST_S, MAX_ROUTINE_ITEMS, MAX_SETS, ROUTINE_COLORS,
};

const MAX_WEIGHT_KG: f64 = 1000.0;
const MAX_REPS: f64 = 10000.0;
const MAX_FOCUS_CHARS: usize = 160;
const MAX_REST_ADJUST_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetField {
    Weight,
    Reps,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum GymMutation {
    CreateRoutine,
    DuplicateRoutine { routine_id: String },
    DeleteRoutine { routine_id: String },
    RenameRoutine { routine_id: String, name: String },
    SetFocus { routine_id: String, focus: String },
    ToggleDay { routine_id: String, day: u8 },
    AddExercise { routine_id: String, exercise_id: String },
    RemoveItem { routine_id: String, item_id: String },
    AddSet { routine_id: String, item_id: String },
    RemoveSet { routine_id: String, item_id: String, index: usize },
    SetValue { routine_id: String, item_id: String, index: usize, field: SetField, value: String },
    AdjustRest { routine_id: String, item_id: String, delta_s: i32 },
    ToggleEquipment { equipment_id: String },
    ApplyPreset { preset: String },
    StartSession { routine_id: String },
    PauseSession,
    ResumeSession,
    FinishSession,
    RestartSession,
    ToggleSetDone { routine_id: String, item_id: String, index: usize },
    AdjustRestTimer { delta_ms: i64 },
    SkipRest,
    /// Una rutina entera, nueva o cambiada (herramientas de la IA). `items`,
    /// si viene, reemplaza los ejercicios: los que ya estaban conservan su id.
    SaveRoutine {
        routine_id: Option<String>,
        name: Option<String>,
        focus: Option<String>,
        days: Option<Vec<u8>>,
        color: Option<String>,
        items: Option<Vec<ItemInput>>,
    },
    /// Un entrenamiento hecho sin la sesión de la pantalla.
    LogWorkout { routine_id: Option<String>, date: String, minutes: f64, exercises: Vec<WorkoutExercise> },
    DeleteWorkout { workout_id: String },
    /// Marca y desmarca equipamiento con el que cuenta.
    SetEquipment { add: Vec<String>, remove: Vec<String> },
    /// Los entrenamientos que registró el reloj (Health Connect).
    ImportWatchSessions { sessions: Vec<WatchSession> },
}

/// Un entrenamiento del reloj. `kind` es la clave del tipo en Health
/// Connect («running», «strength_training»…); `date`, la fecha local del
/// comienzo.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchSession {
    pub id: String,
    pub kind: String,
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub kcal: Option<f64>,
    #[serde(default)]
    pub avg_bpm: Option<f64>,
    #[serde(default)]
    pub max_bpm: Option<f64>,
}

/// Entrenamientos por importación y hasta cuándo para atrás.
pub const MAX_WATCH_SESSIONS: usize = 200;
const WATCH_OLDEST_DAYS: i64 = 60;

/// Un ejercicio de una rutina guardada entera.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInput {
    pub exercise_id: String,
    #[serde(default)]
    pub rest_s: Option<u32>,
    #[serde(default)]
    pub sets: Option<Vec<SetPlan>>,
}

/// Lo que se guarda.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreOp {
    PutRoutine(Routine),
    DeleteRoutine(String),
    PutOwned(Vec<String>),
    PutSession(Option<Session>),
    PutWorkout(Workout),
    DeleteWorkout(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    pub ops: Vec<StoreOp>,
    /// La rutina que la pantalla muestra después (una creada o duplicada).
    pub routine_id: Option<String>,
}

/// Fecha y hora del cambio, y cómo nombrar lo nuevo.
pub struct Clock<'a> {
    pub now_ms: i64,
    /// Fecha local `AAAA-MM-DD`.
    pub today: String,
    pub new_id: &'a mut dyn FnMut() -> String,
}

fn routine<'a>(data: &'a GymData, id: &str) -> GymResult<&'a Routine> {
    data.routine(id).ok_or_else(|| GymError::not_found("La rutina ya no existe."))
}

fn item_mut<'a>(routine: &'a mut Routine, item_id: &str) -> GymResult<&'a mut RoutineItem> {
    routine.items.iter_mut().find(|item| item.id == item_id).ok_or_else(|| GymError::not_found("El ejercicio ya no está en la rutina."))
}

fn put(routine: Routine) -> Change {
    Change { ops: vec![StoreOp::PutRoutine(routine)], routine_id: None }
}

fn text(value: &str, max: usize, what: &str) -> GymResult<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() > max {
        return Err(GymError::validation(format!("{what} puede tener hasta {max} caracteres.")));
    }
    Ok(value)
}

/// Las series con que arranca un ejercicio nuevo en la rutina.
fn default_sets(catalog: &Catalog, exercise_id: &str) -> Vec<SetPlan> {
    let timed = catalog.exercise(exercise_id).is_some_and(|exercise| exercise.tracking.timed());
    vec![SetPlan { weight: None, reps: Some(if timed { 30.0 } else { 10.0 }) }; 3]
}

fn parse_value(value: &str, field: SetField) -> GymResult<Option<f64>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let number = super::parse_number(value).ok_or_else(|| GymError::validation("Escribí un número."))?;
    let max = if field == SetField::Weight { MAX_WEIGHT_KG } else { MAX_REPS };
    if !(0.0..=max).contains(&number) {
        return Err(GymError::validation(match field {
            SetField::Weight => "El peso tiene que estar entre 0 y 1000 kg.",
            SetField::Reps => "Las repeticiones tienen que estar entre 0 y 10000.",
        }));
    }
    Ok(Some((number * 100.0).round() / 100.0))
}

/// Las series que faltan hacer de una rutina con la sesión.
fn pending_sets(routine: &Routine, session: &Session) -> usize {
    routine.items.iter().map(|item| (0..item.sets.len()).filter(|index| !session.is_done(&item.id, *index)).count()).sum()
}

/// El entrenamiento que deja una sesión: las series hechas, con el peso y
/// las repeticiones que tenían. `None` si no hizo ninguna.
pub fn workout_of(routine: &Routine, session: &Session, catalog: &Catalog, now_ms: i64, today: &str, id: String) -> Option<Workout> {
    let mut exercises = Vec::new();
    let mut sets = 0u32;
    let mut volume = 0.0;
    let mut kcal_rates = Vec::new();
    for item in &routine.items {
        let done = item.sets.iter().enumerate().filter(|(index, _)| session.is_done(&item.id, *index)).map(|(_, set)| *set).collect::<Vec<_>>();
        if done.is_empty() {
            continue;
        }
        let exercise = catalog.exercise(&item.exercise);
        let weighted_reps = exercise.is_some_and(|exercise| exercise.tracking.weighted() && !exercise.tracking.timed());
        if weighted_reps {
            volume += done.iter().map(|set| set.weight.unwrap_or(0.0) * set.reps.unwrap_or(0.0)).sum::<f64>();
        }
        kcal_rates.push(exercise.map(|exercise| exercise.kcal_per_min).unwrap_or(5.0));
        sets += done.len() as u32;
        exercises.push(WorkoutExercise { exercise: item.exercise.clone(), sets: done });
    }
    if sets == 0 {
        return None;
    }
    let minutes = ((session.elapsed_ms(now_ms) as f64) / 60000.0).round().max(1.0);
    let rate = kcal_rates.iter().sum::<f64>() / kcal_rates.len() as f64;
    Some(Workout {
        id,
        date: today.to_string(),
        routine_id: routine.id.clone(),
        routine_name: routine.name.clone(),
        color: routine.color.clone(),
        ended_ms: now_ms,
        minutes,
        kcal: (minutes * rate).round(),
        sets,
        volume: (volume * 10.0).round() / 10.0,
        exercises,
        watch_id: None,
        avg_bpm: None,
        max_bpm: None,
    })
}

fn next_color(data: &GymData) -> String {
    ROUTINE_COLORS[data.routines.len() % ROUTINE_COLORS.len()].to_string()
}

fn next_position(data: &GymData) -> i64 {
    data.routines.iter().map(|routine| routine.position).max().map_or(0, |position| position + 1)
}

/// La sesión de una rutina; otra rutina en curso no se pisa.
fn session_for(data: &GymData, routine_id: &str, now_ms: i64) -> GymResult<Session> {
    match &data.session {
        Some(session) if session.active() && session.routine_id != routine_id => {
            let name = data.routine(&session.routine_id).map(|routine| routine.name.as_str()).unwrap_or("otra rutina");
            Err(GymError::validation(format!("Terminá primero el entrenamiento de {name}.")))
        }
        Some(session) if session.active() => Ok(session.clone()),
        _ => Ok(Session {
            routine_id: routine_id.to_string(),
            status: SessionStatus::Running,
            started_ms: now_ms,
            acc_ms: 0,
            rest_end_ms: None,
            rest_left_ms: None,
            rest_total_ms: 0,
            done: Default::default(),
        }),
    }
}

fn active_session(data: &GymData) -> GymResult<Session> {
    data.session.clone().filter(Session::active).ok_or_else(|| GymError::validation("No hay un entrenamiento en curso."))
}

pub fn plan_change(data: &GymData, catalog: &Catalog, mutation: &GymMutation, clock: &mut Clock<'_>) -> GymResult<Change> {
    let now = clock.now_ms;
    Ok(match mutation {
        GymMutation::CreateRoutine => {
            let id = (clock.new_id)();
            let routine = Routine {
                id: id.clone(),
                name: "Nueva rutina".to_string(),
                focus: String::new(),
                days: Vec::new(),
                color: next_color(data),
                position: next_position(data),
                items: Vec::new(),
            };
            Change { ops: vec![StoreOp::PutRoutine(routine)], routine_id: Some(id) }
        }
        GymMutation::DuplicateRoutine { routine_id } => {
            let source = routine(data, routine_id)?;
            let id = (clock.new_id)();
            let items = source.items.iter().map(|item| RoutineItem { id: (clock.new_id)(), ..item.clone() }).collect();
            let copy = Routine {
                id: id.clone(),
                name: format!("{} (copia)", source.name).chars().take(MAX_NAME_CHARS).collect(),
                focus: source.focus.clone(),
                days: Vec::new(),
                color: next_color(data),
                position: next_position(data),
                items,
            };
            Change { ops: vec![StoreOp::PutRoutine(copy)], routine_id: Some(id) }
        }
        GymMutation::DeleteRoutine { routine_id } => {
            routine(data, routine_id)?;
            let mut ops = vec![StoreOp::DeleteRoutine(routine_id.clone())];
            if data.session.as_ref().is_some_and(|session| &session.routine_id == routine_id) {
                ops.push(StoreOp::PutSession(None));
            }
            let next = data.routines.iter().find(|other| &other.id != routine_id).map(|other| other.id.clone());
            Change { ops, routine_id: next }
        }
        GymMutation::RenameRoutine { routine_id, name } => {
            let name = text(name, MAX_NAME_CHARS, "El nombre")?;
            let mut routine = routine(data, routine_id)?.clone();
            routine.name = if name.is_empty() { "Sin nombre".to_string() } else { name };
            put(routine)
        }
        GymMutation::SetFocus { routine_id, focus } => {
            let mut routine = routine(data, routine_id)?.clone();
            routine.focus = text(focus, MAX_FOCUS_CHARS, "El enfoque")?;
            put(routine)
        }
        GymMutation::ToggleDay { routine_id, day } => {
            if *day > 6 {
                return Err(GymError::validation("El día no es válido."));
            }
            let mut routine = routine(data, routine_id)?.clone();
            if let Some(index) = routine.days.iter().position(|existing| existing == day) {
                routine.days.remove(index);
            } else {
                routine.days.push(*day);
                routine.days.sort_unstable();
            }
            put(routine)
        }
        GymMutation::AddExercise { routine_id, exercise_id } => {
            if catalog.exercise(exercise_id).is_none() {
                return Err(GymError::not_found("El ejercicio ya no está en Gym/exercises."));
            }
            let mut routine = routine(data, routine_id)?.clone();
            if routine.items.len() >= MAX_ROUTINE_ITEMS {
                return Err(GymError::validation(format!("Una rutina puede tener hasta {MAX_ROUTINE_ITEMS} ejercicios.")));
            }
            routine.items.push(RoutineItem {
                id: (clock.new_id)(),
                exercise: exercise_id.clone(),
                rest_s: DEFAULT_REST_S,
                sets: default_sets(catalog, exercise_id),
            });
            put(routine)
        }
        GymMutation::RemoveItem { routine_id, item_id } => {
            let mut routine = routine(data, routine_id)?.clone();
            routine.items.retain(|item| &item.id != item_id);
            put(routine)
        }
        GymMutation::AddSet { routine_id, item_id } => {
            let mut routine = routine(data, routine_id)?.clone();
            let item = item_mut(&mut routine, item_id)?;
            if item.sets.len() >= MAX_SETS {
                return Err(GymError::validation(format!("Un ejercicio puede tener hasta {MAX_SETS} series.")));
            }
            let last = item.sets.last().copied().unwrap_or(SetPlan { weight: None, reps: Some(10.0) });
            item.sets.push(last);
            put(routine)
        }
        GymMutation::RemoveSet { routine_id, item_id, index } => {
            let mut routine = routine(data, routine_id)?.clone();
            let item = item_mut(&mut routine, item_id)?;
            if item.sets.len() <= 1 {
                return Err(GymError::validation("El ejercicio necesita al menos una serie."));
            }
            if *index >= item.sets.len() {
                return Err(GymError::not_found("La serie ya no existe."));
            }
            item.sets.remove(*index);
            put(routine)
        }
        GymMutation::SetValue { routine_id, item_id, index, field, value } => {
            let parsed = parse_value(value, *field)?;
            let mut routine = routine(data, routine_id)?.clone();
            let item = item_mut(&mut routine, item_id)?;
            let set = item.sets.get_mut(*index).ok_or_else(|| GymError::not_found("La serie ya no existe."))?;
            match field {
                SetField::Weight => set.weight = parsed,
                SetField::Reps => set.reps = parsed,
            }
            put(routine)
        }
        GymMutation::AdjustRest { routine_id, item_id, delta_s } => {
            let mut routine = routine(data, routine_id)?.clone();
            let item = item_mut(&mut routine, item_id)?;
            let next = (item.rest_s as i64 + *delta_s as i64).clamp(0, MAX_REST_S as i64) as u32;
            item.rest_s = next;
            put(routine)
        }
        GymMutation::ToggleEquipment { equipment_id } => {
            if catalog.equipment(equipment_id).is_none() {
                return Err(GymError::not_found("El equipamiento ya no está en Gym/equipment."));
            }
            let mut owned = data.owned.clone();
            if !owned.remove(equipment_id) {
                owned.insert(equipment_id.clone());
            }
            Change { ops: vec![StoreOp::PutOwned(owned.into_iter().collect())], routine_id: None }
        }
        GymMutation::ApplyPreset { preset } => {
            // Lo agregado por la persona que ya tenía marcado se mantiene.
            let mut owned = catalog
                .equipment
                .iter()
                .filter(|item| item.custom && data.owned.contains(&item.id))
                .map(|item| item.id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            match preset.as_str() {
                "gym" => owned.extend(catalog.equipment.iter().filter(|item| !item.custom).map(|item| item.id.clone())),
                "casa" => owned.extend(HOME_PRESET.iter().filter(|id| catalog.equipment(id).is_some()).map(|id| id.to_string())),
                "nada" => {}
                _ => return Err(GymError::validation("El atajo no existe.")),
            }
            Change { ops: vec![StoreOp::PutOwned(owned.into_iter().collect())], routine_id: None }
        }
        GymMutation::StartSession { routine_id } => {
            routine(data, routine_id)?;
            let session = session_for(data, routine_id, now)?;
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::PauseSession => {
            let mut session = active_session(data)?;
            if session.status == SessionStatus::Running {
                session.acc_ms = session.elapsed_ms(now);
                session.rest_left_ms = session.rest_end_ms.filter(|end| *end > now).map(|end| end - now);
                session.rest_end_ms = None;
                session.status = SessionStatus::Paused;
                session.started_ms = now;
            }
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::ResumeSession => {
            let mut session = active_session(data)?;
            if session.status == SessionStatus::Paused {
                session.status = SessionStatus::Running;
                session.started_ms = now;
                session.rest_end_ms = session.rest_left_ms.take().map(|left| now + left);
            }
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::FinishSession => {
            let mut session = active_session(data)?;
            let mut ops = Vec::new();
            if let Some(routine) = data.routine(&session.routine_id) {
                if let Some(workout) = workout_of(routine, &session, catalog, now, &clock.today, (clock.new_id)()) {
                    ops.push(StoreOp::PutWorkout(workout));
                }
            }
            session.acc_ms = session.elapsed_ms(now);
            session.status = SessionStatus::Done;
            session.started_ms = now;
            session.rest_end_ms = None;
            session.rest_left_ms = None;
            ops.push(StoreOp::PutSession(Some(session)));
            Change { ops, routine_id: None }
        }
        GymMutation::RestartSession => Change { ops: vec![StoreOp::PutSession(None)], routine_id: None },
        GymMutation::ToggleSetDone { routine_id, item_id, index } => {
            let routine = routine(data, routine_id)?;
            let item = routine.items.iter().find(|item| &item.id == item_id).ok_or_else(|| GymError::not_found("El ejercicio ya no está en la rutina."))?;
            if *index >= item.sets.len() {
                return Err(GymError::not_found("La serie ya no existe."));
            }
            // Un entrenamiento terminado queda terminado: un toque que llega
            // tarde (la pantalla se trabó y los toques se encolaron) no arranca
            // otro desde cero encima.
            if data.session.as_ref().is_some_and(|session| session.status == SessionStatus::Done && &session.routine_id == routine_id) {
                return Err(GymError::validation("Ya terminaste este entrenamiento. Tocá «Empezar de nuevo» para entrenar otra vez."));
            }
            let mut session = session_for(data, routine_id, now)?;
            let was_done = session.is_done(item_id, *index);
            if session.status == SessionStatus::Paused && !was_done {
                session.status = SessionStatus::Running;
                session.started_ms = now;
                session.rest_left_ms = None;
            }
            let flags = session.done.entry(item_id.clone()).or_insert_with(Vec::new);
            if flags.len() < item.sets.len() {
                flags.resize(item.sets.len(), false);
            }
            flags[*index] = !was_done;
            if !was_done {
                let rest_ms = item.rest_s as i64 * 1000;
                if pending_sets(routine, &session) > 0 && rest_ms > 0 {
                    session.rest_total_ms = rest_ms;
                    session.rest_end_ms = Some(now + rest_ms);
                    session.rest_left_ms = None;
                } else {
                    session.rest_end_ms = None;
                    session.rest_left_ms = None;
                }
            }
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::AdjustRestTimer { delta_ms } => {
            let mut session = active_session(data)?;
            let delta = (*delta_ms).clamp(-MAX_REST_ADJUST_MS, MAX_REST_ADJUST_MS);
            if let Some(end) = session.rest_end_ms {
                let end = (end + delta).max(now);
                session.rest_end_ms = Some(end);
                session.rest_total_ms = session.rest_total_ms.max(end - now);
            } else if let Some(left) = session.rest_left_ms {
                let left = (left + delta).max(0);
                session.rest_left_ms = Some(left);
                session.rest_total_ms = session.rest_total_ms.max(left);
            }
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::SkipRest => {
            let mut session = active_session(data)?;
            session.rest_end_ms = None;
            session.rest_left_ms = None;
            Change { ops: vec![StoreOp::PutSession(Some(session))], routine_id: None }
        }
        GymMutation::SaveRoutine { routine_id, name, focus, days, color, items } => {
            let mut routine = match routine_id {
                Some(id) => routine(data, id)?.clone(),
                None => Routine {
                    id: (clock.new_id)(),
                    name: "Nueva rutina".to_string(),
                    focus: String::new(),
                    days: Vec::new(),
                    color: next_color(data),
                    position: next_position(data),
                    items: Vec::new(),
                },
            };
            if let Some(name) = name {
                let name = text(name, MAX_NAME_CHARS, "El nombre")?;
                routine.name = if name.is_empty() { "Sin nombre".to_string() } else { name };
            }
            if let Some(focus) = focus {
                routine.focus = text(focus, MAX_FOCUS_CHARS, "El enfoque")?;
            }
            if let Some(days) = days {
                if days.iter().any(|day| *day > 6) {
                    return Err(GymError::validation("Los días van de 0 (lunes) a 6 (domingo)."));
                }
                let mut days = days.clone();
                days.sort_unstable();
                days.dedup();
                routine.days = days;
            }
            if let Some(color) = color {
                if !ROUTINE_COLORS.contains(&color.as_str()) {
                    return Err(GymError::validation(format!("El color tiene que ser uno de: {}.", ROUTINE_COLORS.join(", "))));
                }
                routine.color = color.clone();
            }
            if let Some(items) = items {
                routine.items = routine_items(&routine.items, items, catalog, clock)?;
            }
            let id = routine.id.clone();
            Change { ops: vec![StoreOp::PutRoutine(routine)], routine_id: Some(id) }
        }
        GymMutation::LogWorkout { routine_id, date, minutes, exercises } => {
            let workout = logged_workout(data, catalog, routine_id.as_deref(), date, *minutes, exercises, clock)?;
            Change { ops: vec![StoreOp::PutWorkout(workout)], routine_id: None }
        }
        GymMutation::DeleteWorkout { workout_id } => {
            if !data.workouts.iter().any(|workout| &workout.id == workout_id) {
                return Err(GymError::not_found("El entrenamiento ya no está en el historial."));
            }
            Change { ops: vec![StoreOp::DeleteWorkout(workout_id.clone())], routine_id: None }
        }
        GymMutation::SetEquipment { add, remove } => {
            let mut owned = data.owned.clone();
            for id in add.iter().chain(remove) {
                if catalog.equipment(id).is_none() {
                    return Err(GymError::not_found(format!("El equipamiento «{id}» no está en Gym/equipment.")));
                }
            }
            owned.extend(add.iter().cloned());
            for id in remove {
                owned.remove(id);
            }
            Change { ops: vec![StoreOp::PutOwned(owned.into_iter().collect())], routine_id: None }
        }
        GymMutation::ImportWatchSessions { sessions } => import_watch_sessions(data, sessions, clock)?,
    })
}

/// Los entrenamientos del reloj. Uno que se superpone (la mitad o más del
/// más corto) con un entrenamiento de Notia lo completa con las calorías y
/// el pulso del reloj, sin cambiar sus series ni sus minutos; uno que no,
/// se suma como entrenamiento propio («Correr», «Fuerza»…). Importar de
/// nuevo actualiza en lugar de duplicar. Lo que no se entiende (sin tipo
/// conocido, menos de 5 minutos o más de 6 horas, fechas raras) se ignora.
fn import_watch_sessions(data: &GymData, sessions: &[WatchSession], clock: &mut Clock) -> GymResult<Change> {
    if sessions.len() > MAX_WATCH_SESSIONS {
        return Err(GymError::validation(format!("Se importan hasta {MAX_WATCH_SESSIONS} entrenamientos por vez.")));
    }
    let today = crate::health::parse_date(&clock.today);
    let oldest = today.map(|today| crate::health::days_ago(today, WATCH_OLDEST_DAYS));
    let mut workouts = data.workouts.clone();
    let mut ops = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for session in sessions {
        let Some(label) = super::watch_session_label(session.kind.trim()) else { continue };
        let minutes = (session.end_ms - session.start_ms) as f64 / 60_000.0;
        let date = crate::health::parse_date(session.date.trim());
        let dated = date.zip(today).zip(oldest).is_some_and(|((date, today), oldest)| date <= today && date >= oldest);
        if session.id.trim().is_empty() || !seen.insert(session.id.trim().to_string()) || !(5.0..=360.0).contains(&minutes) || !dated {
            continue;
        }
        let kcal = session.kcal.filter(|kcal| (0.0..=3_000.0).contains(kcal)).map(f64::round);
        let bpm = |value: Option<f64>| value.filter(|bpm| (30.0..=230.0).contains(bpm)).map(f64::round);
        let (avg_bpm, max_bpm) = (bpm(session.avg_bpm), bpm(session.max_bpm));
        let watch_id = Some(session.id.trim().to_string());
        let overlap = |workout: &Workout| {
            let start = workout.ended_ms - (workout.minutes * 60_000.0) as i64;
            let shared = (workout.ended_ms.min(session.end_ms) - start.max(session.start_ms)).max(0) as f64;
            let shorter = ((workout.ended_ms - start).min(session.end_ms - session.start_ms)).max(1) as f64;
            shared / shorter
        };
        let index = workouts.iter().position(|workout| workout.watch_id == watch_id).or_else(|| {
            workouts
                .iter()
                .enumerate()
                .filter(|(_, workout)| workout.watch_id.is_none() && overlap(workout) >= 0.5)
                .max_by(|(_, a), (_, b)| overlap(a).total_cmp(&overlap(b)))
                .map(|(index, _)| index)
        });
        let updated = match index {
            // Uno de Notia: suma lo del reloj y conserva el resto.
            Some(index) if !workouts[index].routine_id.is_empty() || !workouts[index].exercises.is_empty() => {
                let mut workout = workouts[index].clone();
                workout.watch_id = watch_id;
                workout.kcal = kcal.unwrap_or(workout.kcal);
                workout.avg_bpm = avg_bpm.or(workout.avg_bpm);
                workout.max_bpm = max_bpm.or(workout.max_bpm);
                (index, workout)
            }
            // Uno del reloj ya importado: lo que traiga ahora.
            Some(index) => {
                let mut workout = workouts[index].clone();
                workout.date = session.date.trim().to_string();
                workout.routine_name = label.to_string();
                workout.ended_ms = session.end_ms;
                workout.minutes = crate::health::round1(minutes);
                workout.kcal = kcal.unwrap_or(workout.kcal);
                workout.avg_bpm = avg_bpm;
                workout.max_bpm = max_bpm;
                (index, workout)
            }
            None => {
                let workout = Workout {
                    id: (clock.new_id)(),
                    date: session.date.trim().to_string(),
                    routine_id: String::new(),
                    routine_name: label.to_string(),
                    color: "teal".to_string(),
                    ended_ms: session.end_ms,
                    minutes: crate::health::round1(minutes),
                    kcal: kcal.unwrap_or(0.0),
                    sets: 0,
                    volume: 0.0,
                    exercises: Vec::new(),
                    watch_id,
                    avg_bpm,
                    max_bpm,
                };
                workouts.push(workout.clone());
                ops.push(StoreOp::PutWorkout(workout));
                continue;
            }
        };
        let (index, workout) = updated;
        if workouts[index] != workout {
            workouts[index] = workout.clone();
            ops.push(StoreOp::PutWorkout(workout));
        }
    }
    Ok(Change { ops, routine_id: None })
}

fn checked_sets(sets: &[SetPlan]) -> GymResult<Vec<SetPlan>> {
    if sets.is_empty() || sets.len() > MAX_SETS {
        return Err(GymError::validation(format!("Cada ejercicio lleva de 1 a {MAX_SETS} series.")));
    }
    sets.iter()
        .map(|set| {
            let weight = set.weight.filter(|weight| *weight > 0.0);
            if weight.is_some_and(|weight| weight > MAX_WEIGHT_KG) || set.reps.is_some_and(|reps| !(0.0..=MAX_REPS).contains(&reps)) || set.weight.is_some_and(|weight| weight < 0.0) {
                return Err(GymError::validation("El peso va de 0 a 1000 kg y las repeticiones de 0 a 10000."));
            }
            Ok(SetPlan { weight: weight.map(|weight| (weight * 100.0).round() / 100.0), reps: set.reps.map(|reps| (reps * 100.0).round() / 100.0) })
        })
        .collect()
}

/// Los ejercicios de una rutina guardada entera: los que ya estaban
/// conservan su id (y las series hechas de la sesión).
fn routine_items(current: &[RoutineItem], inputs: &[ItemInput], catalog: &Catalog, clock: &mut Clock<'_>) -> GymResult<Vec<RoutineItem>> {
    if inputs.len() > MAX_ROUTINE_ITEMS {
        return Err(GymError::validation(format!("Una rutina puede tener hasta {MAX_ROUTINE_ITEMS} ejercicios.")));
    }
    let mut used = Vec::new();
    inputs
        .iter()
        .map(|input| {
            if catalog.exercise(&input.exercise_id).is_none() {
                return Err(GymError::not_found(format!("El ejercicio «{}» no está en Gym/exercises.", input.exercise_id)));
            }
            let existing = current.iter().find(|item| item.exercise == input.exercise_id && !used.contains(&item.id));
            let id = existing.map(|item| item.id.clone()).unwrap_or_else(|| (clock.new_id)());
            used.push(id.clone());
            let sets = match &input.sets {
                Some(sets) => checked_sets(sets)?,
                None => existing.map(|item| item.sets.clone()).unwrap_or_else(|| default_sets(catalog, &input.exercise_id)),
            };
            let rest_s = input.rest_s.map(|rest| rest.min(MAX_REST_S)).or(existing.map(|item| item.rest_s)).unwrap_or(DEFAULT_REST_S);
            Ok(RoutineItem { id, exercise: input.exercise_id.clone(), rest_s, sets })
        })
        .collect()
}

const MAX_WORKOUT_MINUTES: f64 = 600.0;

/// Un entrenamiento contado por la persona: las series que dijo, con sus
/// minutos y las calorías aproximadas.
fn logged_workout(
    data: &GymData,
    catalog: &Catalog,
    routine_id: Option<&str>,
    date: &str,
    minutes: f64,
    exercises: &[WorkoutExercise],
    clock: &mut Clock<'_>,
) -> GymResult<Workout> {
    let day = super::parse_date(date).ok_or_else(|| GymError::validation("La fecha tiene que ser AAAA-MM-DD."))?;
    let today = super::parse_date(&clock.today).ok_or_else(|| GymError::validation("La fecha de hoy no es válida."))?;
    if day > today {
        return Err(GymError::validation("La fecha no puede ser futura."));
    }
    if !(1.0..=MAX_WORKOUT_MINUTES).contains(&minutes) {
        return Err(GymError::validation("Los minutos van de 1 a 600."));
    }
    if exercises.is_empty() {
        return Err(GymError::validation("Contá al menos un ejercicio con sus series."));
    }
    let routine = routine_id.map(|id| routine(data, id)).transpose()?;
    let mut sets = 0u32;
    let mut volume = 0.0;
    let mut rates = Vec::new();
    let mut done = Vec::new();
    for item in exercises {
        let exercise = catalog.exercise(&item.exercise).ok_or_else(|| GymError::not_found(format!("El ejercicio «{}» no está en Gym/exercises.", item.exercise)))?;
        let item_sets = checked_sets(&item.sets)?;
        if exercise.tracking.weighted() && !exercise.tracking.timed() {
            volume += item_sets.iter().map(|set| set.weight.unwrap_or(0.0) * set.reps.unwrap_or(0.0)).sum::<f64>();
        }
        rates.push(exercise.kcal_per_min);
        sets += item_sets.len() as u32;
        done.push(WorkoutExercise { exercise: item.exercise.clone(), sets: item_sets });
    }
    let days_ago = today.since(day).map(|span| span.get_days() as i64).unwrap_or(0);
    let minutes = minutes.round();
    Ok(Workout {
        id: (clock.new_id)(),
        date: day.to_string(),
        routine_id: routine.map(|routine| routine.id.clone()).unwrap_or_default(),
        routine_name: routine.map(|routine| routine.name.clone()).unwrap_or_else(|| "Entrenamiento libre".to_string()),
        color: routine.map(|routine| routine.color.clone()).unwrap_or_else(|| "teal".to_string()),
        ended_ms: clock.now_ms - days_ago * 86_400_000,
        minutes,
        kcal: (minutes * rates.iter().sum::<f64>() / rates.len() as f64).round(),
        sets,
        volume: (volume * 10.0).round() / 10.0,
        exercises: done,
        watch_id: None,
        avg_bpm: None,
        max_bpm: None,
    })
}

// ---------------------------------------------------------------------------
// Catálogo: la ficha de un ejercicio y el equipamiento propio
// ---------------------------------------------------------------------------

/// Un cambio de la ficha de un ejercicio.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "field", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum ExerciseEdit {
    Name { value: String },
    Group { value: String },
    Weighted { value: bool },
    Timed { value: bool },
    /// Ninguno → principal → secundario → ninguno.
    CycleMuscle { muscle: String },
    ToggleEquipment { equipment_id: String },
    Kcal { value: String },
    KcalStep { delta: f64 },
    Step { index: usize, text: String },
    AddStep,
    RemoveStep { index: usize },
    RemoveMedia,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum CatalogMutation {
    CreateExercise { group: Option<String> },
    UpdateExercise { exercise_id: String, edit: ExerciseEdit },
    SaveEquipment { equipment_id: Option<String>, name: String, category: String },
    DeleteEquipment { equipment_id: String },
}

const MAX_KCAL_PER_MIN: f64 = 40.0;

/// Aplica un cambio de la ficha. `exercise` se leyó con su imagen, así el
/// archivo se reescribe sin perderla.
pub fn apply_exercise_edit(exercise: &mut super::Exercise, edit: &ExerciseEdit, catalog: &Catalog) -> GymResult<()> {
    match edit {
        ExerciseEdit::Name { value } => {
            let name = text(value, MAX_NAME_CHARS, "El nombre")?;
            if name.is_empty() {
                return Err(GymError::validation("El ejercicio necesita un nombre."));
            }
            exercise.name = name;
        }
        ExerciseEdit::Group { value } => {
            if !super::is_group(value) {
                return Err(GymError::validation("El grupo no existe."));
            }
            exercise.group = value.clone();
        }
        ExerciseEdit::Weighted { value } => exercise.tracking = exercise.tracking.with_weight(*value),
        ExerciseEdit::Timed { value } => {
            exercise.tracking = match (*value, exercise.tracking.weighted()) {
                (true, true) => super::Tracking::WeightTime,
                (true, false) => super::Tracking::Time,
                (false, true) => super::Tracking::WeightReps,
                (false, false) => super::Tracking::Reps,
            };
        }
        ExerciseEdit::CycleMuscle { muscle } => {
            let key = super::muscle_key(muscle).ok_or_else(|| GymError::validation("El músculo no existe."))?.to_string();
            match exercise.muscle_level(&key) {
                0 => exercise.primary.push(key),
                2 => {
                    exercise.primary.retain(|existing| existing != &key);
                    exercise.secondary.push(key);
                }
                _ => exercise.secondary.retain(|existing| existing != &key),
            }
        }
        ExerciseEdit::ToggleEquipment { equipment_id } => {
            if let Some(index) = exercise.equipment.iter().position(|id| id == equipment_id) {
                exercise.equipment.remove(index);
            } else if catalog.equipment(equipment_id).is_some() {
                exercise.equipment.push(equipment_id.clone());
            } else {
                return Err(GymError::not_found("El equipamiento ya no existe."));
            }
        }
        ExerciseEdit::Kcal { value } => {
            let kcal = super::parse_number(value).ok_or_else(|| GymError::validation("Escribí las calorías por minuto con un número."))?;
            if !(0.0..=MAX_KCAL_PER_MIN).contains(&kcal) {
                return Err(GymError::validation("Las calorías por minuto tienen que estar entre 0 y 40."));
            }
            exercise.kcal_per_min = (kcal * 10.0).round() / 10.0;
        }
        ExerciseEdit::KcalStep { delta } => {
            exercise.kcal_per_min = ((exercise.kcal_per_min + delta).clamp(0.0, MAX_KCAL_PER_MIN) * 10.0).round() / 10.0;
        }
        ExerciseEdit::Step { index, text: value } => {
            let step = text(value, super::MAX_STEP_CHARS, "Un paso")?;
            let slot = exercise.steps.get_mut(*index).ok_or_else(|| GymError::not_found("El paso ya no existe."))?;
            *slot = step;
        }
        ExerciseEdit::AddStep => {
            if exercise.steps.len() >= super::MAX_STEPS {
                return Err(GymError::validation(format!("Un ejercicio puede tener hasta {} pasos.", super::MAX_STEPS)));
            }
            exercise.steps.push(String::new());
        }
        ExerciseEdit::RemoveStep { index } => {
            if *index < exercise.steps.len() {
                exercise.steps.remove(*index);
            }
        }
        ExerciseEdit::RemoveMedia => {
            exercise.image = None;
            exercise.has_image = false;
            exercise.video = None;
        }
    }
    Ok(())
}

/// Calorías por minuto de referencia para un ejercicio nuevo, por grupo.
pub fn default_kcal(group: &str) -> f64 {
    match group {
        "piernas" | "completo" => 7.0,
        "cardio" => 9.0,
        "pecho" | "espalda" => 6.0,
        "hombros" => 5.0,
        "core" => 4.5,
        "brazos" => 3.5,
        "estiramiento" => 2.5,
        _ => 5.0,
    }
}

/// Un ejercicio nuevo, todavía sin guardar. `path_for` da la ruta del
/// archivo para un nombre.
pub fn new_exercise(group: Option<&str>, catalog: &Catalog, path_for: impl Fn(&str) -> String) -> super::Exercise {
    let group = group.filter(|group| super::is_group(group)).unwrap_or("pecho").to_string();
    let mut name = "Ejercicio nuevo".to_string();
    let mut number = 2;
    while catalog.exercises.iter().any(|exercise| super::fold(&exercise.name) == super::fold(&name)) {
        name = format!("Ejercicio nuevo {number}");
        number += 1;
    }
    let mut id = format!("propio_{}", super::slug_id(&name));
    while catalog.exercise(&id).is_some() {
        id.push('_');
    }
    super::Exercise {
        id,
        path: path_for(&name),
        name,
        kcal_per_min: default_kcal(&group),
        group,
        tracking: super::Tracking::WeightReps,
        ..super::Exercise::default()
    }
}

/// Un equipamiento propio nuevo o editado. La foto la pone el adaptador.
pub fn saved_equipment(
    existing: Option<&super::Equipment>,
    name: &str,
    category: &str,
    catalog: &Catalog,
    path_for: impl Fn(&str) -> String,
) -> GymResult<super::Equipment> {
    let name = text(name, MAX_NAME_CHARS, "El nombre")?;
    if name.is_empty() {
        return Err(GymError::validation("El equipamiento necesita un nombre."));
    }
    if !super::is_category(category) {
        return Err(GymError::validation("La categoría no existe."));
    }
    let duplicate = catalog
        .equipment
        .iter()
        .any(|item| super::fold(&item.name) == super::fold(&name) && existing.is_none_or(|existing| existing.id != item.id));
    if duplicate {
        return Err(GymError::validation(format!("Ya existe «{name}» en el equipamiento.")));
    }
    match existing {
        Some(existing) if !existing.custom => Err(GymError::validation("El equipamiento del catálogo no se edita desde acá.")),
        Some(existing) => Ok(super::Equipment { name, category: category.to_string(), ..existing.clone() }),
        None => {
            let mut id = format!("propio_{}", super::slug_id(&name));
            while catalog.equipment(&id).is_some() {
                id.push('_');
            }
            Ok(super::Equipment { id, path: path_for(&name), name, category: category.to_string(), custom: true, ..super::Equipment::default() })
        }
    }
}
