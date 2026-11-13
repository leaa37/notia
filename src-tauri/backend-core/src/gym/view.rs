//! Lo que muestra cada pantalla de Rutinas, calculado con los datos del
//! usuario y el catálogo. La pantalla solo pinta: los colores de los grupos y
//! de los estados los pone el tema; los relojes los cuenta con los tiempos
//! de la sesión.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::health::shift_days;

use super::{
    category_label, count, fmt, fold, group_label, hours_minutes, lower_first, muscle_label, parse_date, rest_clock, span, Catalog,
    Equipment, Exercise, GymData, Routine, RoutineItem, Session, SessionStatus, SetPlan, Tracking, Workout, EQUIPMENT_CATEGORIES,
    GROUPS, HOME_PRESET, MUSCLES, PRESETS,
};

const HEAT_WEEKS: i64 = 24;
const BAR_DAYS: i64 = 14;
const BAR_MAX_PX: f64 = 112.0;
const FATIGUED_HOURS: f64 = 36.0;
const RECOVERING_HOURS: f64 = 72.0;
const DAY_NAMES: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const DAY_SHORT: [&str; 7] = ["Lun", "Mar", "Mié", "Jue", "Vie", "Sáb", "Dom"];
const DAY_LETTERS: [&str; 7] = ["L", "M", "X", "J", "V", "S", "D"];
const MONTHS: [&str; 12] = ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"];
const MONTHS_SHORT: [&str; 12] = ["Ene", "Feb", "Mar", "Abr", "May", "Jun", "Jul", "Ago", "Sep", "Oct", "Nov", "Dic"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Screen {
    #[default]
    Panel,
    Rutinas,
    Editar,
    Entrenar,
    Equipo,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GymQuery {
    #[serde(default)]
    pub screen: Screen,
    #[serde(default)]
    pub routine_id: Option<String>,
    /// Día elegido en el calendario, `AAAA-MM-DD`.
    #[serde(default)]
    pub day: Option<String>,
    #[serde(default)]
    pub search: String,
    /// `None` o `todos`: todos los grupos.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default = "yes")]
    pub only_mine: bool,
    /// Al abrir Gimnasio: un entrenamiento en curso vuelve a su pantalla.
    #[serde(default)]
    pub resume: bool,
}

impl Default for GymQuery {
    fn default() -> Self {
        Self { screen: Screen::Panel, routine_id: None, day: None, search: String::new(), group: None, only_mine: true, resume: false }
    }
}

// ---------------------------------------------------------------------------
// Vista
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRow {
    pub id: String,
    pub name: String,
    pub focus: String,
    pub count: String,
    pub days: String,
    pub color: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBadge {
    pub routine_id: String,
    pub routine_name: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GymView {
    /// La pantalla armada: la pedida, o Entrenar al retomar un entrenamiento.
    pub screen: Screen,
    pub sex: String,
    pub sex_from_profile: bool,
    pub routines: Vec<RoutineRow>,
    pub routine_id: Option<String>,
    pub equipment_owned: usize,
    pub equipment_total: usize,
    pub exercise_total: usize,
    pub session: Option<SessionBadge>,
    pub groups: Vec<Choice>,
    pub panel: Option<PanelView>,
    pub routine: Option<RoutineView>,
    pub library: Option<LibraryView>,
    pub training: Option<TrainingView>,
    pub equipment: Option<EquipmentView>,
    /// Hora del servidor, para que el reloj de la pantalla no dependa del
    /// reloj del dispositivo que la muestra.
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatCard {
    pub key: String,
    pub value: String,
    pub sub: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuscleRow {
    pub key: String,
    pub name: String,
    /// `fat`, `rec`, `ok` o `none`.
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegendRow {
    pub state: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatDay {
    pub date: String,
    pub color: Option<String>,
    /// 0 sin entrenamiento; 1 a 3, de menos a más calorías.
    pub level: u8,
    pub future: bool,
    pub today: bool,
    pub selected: bool,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatWeek {
    pub month: String,
    pub days: Vec<HeatDay>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayDetail {
    pub date: String,
    pub has: bool,
    pub routine: String,
    pub color: Option<String>,
    pub kcal: String,
    pub minutes: String,
    pub sets: String,
    pub none_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegendColor {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bar {
    pub letter: String,
    pub day: u32,
    pub value: String,
    pub height: u32,
    pub color: Option<String>,
    pub today: bool,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelView {
    pub week_label: String,
    pub next_label: String,
    pub next_routine_id: Option<String>,
    pub next_name: String,
    /// La próxima es la rutina asignada a hoy y hoy todavía no entrenó: la
    /// que se recomienda empezar.
    pub next_is_today: bool,
    pub stats: Vec<StatCard>,
    pub muscle_states: BTreeMap<String, String>,
    pub muscles: Vec<MuscleRow>,
    pub legend: Vec<LegendRow>,
    pub advice: String,
    pub weeks: Vec<HeatWeek>,
    pub day: DayDetail,
    pub routine_legend: Vec<LegendColor>,
    pub bars: Vec<Bar>,
    pub calories_14: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetView {
    pub weight: String,
    pub reps: String,
    pub weight_label: String,
    pub reps_label: String,
    pub volume_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: String,
    pub n: usize,
    pub exercise_id: String,
    pub name: String,
    pub group: String,
    pub group_label: String,
    pub equipment_label: String,
    pub rest_s: u32,
    pub rest_label: String,
    pub missing: Option<String>,
    pub summary: String,
    pub weighted: bool,
    pub timed: bool,
    pub sets: Vec<SetView>,
    pub total_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineStats {
    pub exercises: usize,
    pub sets: usize,
    pub volume: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuscleSummary {
    /// 2 principal, 1 secundario.
    pub muscles: BTreeMap<String, u8>,
    pub primary: String,
    pub secondary: String,
    pub rest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    pub id: String,
    pub name: String,
    pub focus: String,
    pub focus_label: String,
    pub color: String,
    pub days: Vec<bool>,
    pub items: Vec<ItemView>,
    pub stats: RoutineStats,
    pub summary: MuscleSummary,
    pub train_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    pub id: String,
    pub name: String,
    pub group: String,
    pub meta: String,
    pub missing: Option<String>,
    pub added: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub rows: Vec<LibraryRow>,
    pub hidden: usize,
    pub hidden_note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainSet {
    pub weight: String,
    pub reps: String,
    pub done: bool,
    pub next: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainItem {
    pub id: String,
    pub n: usize,
    pub exercise_id: String,
    pub name: String,
    pub group: String,
    pub group_label: String,
    pub rest_label: String,
    pub missing: Option<String>,
    pub weighted: bool,
    pub timed: bool,
    pub count: String,
    pub all_done: bool,
    pub current: bool,
    pub sets: Vec<TrainSet>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingView {
    pub routine_id: String,
    pub name: String,
    pub focus_label: String,
    /// `idle`, `running`, `paused` o `done`.
    pub status: String,
    pub started_ms: i64,
    pub acc_ms: i64,
    pub rest_end_ms: Option<i64>,
    pub rest_left_ms: Option<i64>,
    pub rest_total_ms: i64,
    pub done_sets: usize,
    pub total_sets: usize,
    pub percent: u32,
    pub volume: String,
    pub kcal_per_min: f64,
    pub items: Vec<TrainItem>,
    pub next_text: Option<String>,
    pub current_rest: Option<String>,
    pub done_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentRow {
    pub id: String,
    pub name: String,
    pub category: String,
    pub category_label: String,
    pub sub: String,
    pub owned: bool,
    pub custom: bool,
    pub has_image: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetChip {
    pub id: String,
    pub label: String,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub exercise: String,
    pub need: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineImpact {
    pub name: String,
    pub color: String,
    pub ok: bool,
    pub status: String,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentView {
    pub items: Vec<EquipmentRow>,
    pub presets: Vec<PresetChip>,
    pub available: String,
    pub available_percent: u32,
    pub routines: Vec<RoutineImpact>,
    pub categories: Vec<Choice>,
}

// ---------------------------------------------------------------------------
// Ayudas
// ---------------------------------------------------------------------------

fn weekday(date: Date) -> usize {
    date.weekday().to_monday_zero_offset() as usize
}

fn monday_of(date: Date) -> Date {
    shift_days(date, -(weekday(date) as i64))
}

fn key(date: Date) -> String {
    date.to_string()
}

/// «Lunes 3 de agosto».
fn long_day(date: Date) -> String {
    let name = DAY_NAMES[weekday(date)];
    let mut chars = name.chars();
    let capital = chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
    format!("{capital} {} de {}", date.day(), MONTHS[date.month() as usize - 1])
}

/// Un número para un campo de texto: «62,5» o vacío.
fn input_value(value: Option<f64>) -> String {
    value.map(|number| fmt(number, 2)).unwrap_or_default()
}

/// Lo que falta del equipamiento que necesita un ejercicio (solo lo que
/// existe en el catálogo).
pub fn missing_equipment(exercise: &Exercise, catalog: &Catalog, owned: &BTreeSet<String>) -> Vec<String> {
    exercise
        .equipment
        .iter()
        .filter(|id| !owned.contains(*id))
        .filter_map(|id| catalog.equipment_name(id))
        .map(str::to_string)
        .collect()
}

fn missing_text(missing: &[String]) -> Option<String> {
    (!missing.is_empty()).then(|| format!("Necesita {}, que no tenés marcado", missing.iter().map(|name| lower_first(name)).collect::<Vec<_>>().join(", ")))
}

fn equipment_label(exercise: &Exercise, catalog: &Catalog) -> String {
    let names = exercise.equipment.iter().filter_map(|id| catalog.equipment_name(id)).collect::<Vec<_>>();
    if names.is_empty() {
        "Sin equipo".to_string()
    } else {
        names.join(", ")
    }
}

/// El ejercicio de un ítem, o uno vacío si se borró su archivo.
fn exercise_of<'a>(catalog: &'a Catalog, id: &str, fallback: &'a Exercise) -> &'a Exercise {
    catalog.exercise(id).unwrap_or(fallback)
}

fn removed_exercise(id: &str) -> Exercise {
    Exercise { id: id.to_string(), name: "Ejercicio eliminado".to_string(), group: "core".to_string(), ..Exercise::default() }
}

fn unit_label(value: f64, timed: bool) -> String {
    if timed {
        format!("{} s", fmt(value, 1))
    } else {
        format!("{} reps", fmt(value, 1))
    }
}

fn set_view(set: &SetPlan, tracking: Tracking) -> SetView {
    let reps = set.reps.unwrap_or(0.0);
    let weight = set.weight.unwrap_or(0.0);
    let weight_label = if !tracking.weighted() {
        "Peso corporal".to_string()
    } else if weight > 0.0 {
        format!("{} kg", fmt(weight, 2))
    } else {
        "Sin peso".to_string()
    };
    let volume_label = if tracking.weighted() && !tracking.timed() { format!("{} kg", fmt(weight * reps, 1)) } else { unit_label(reps, tracking.timed()) };
    SetView { weight: input_value(set.weight), reps: input_value(set.reps), weight_label, reps_label: unit_label(reps, tracking.timed()), volume_label }
}

fn item_volume(item: &RoutineItem, tracking: Tracking) -> f64 {
    if tracking.weighted() && !tracking.timed() {
        item.sets.iter().map(|set| set.weight.unwrap_or(0.0) * set.reps.unwrap_or(0.0)).sum()
    } else {
        0.0
    }
}

fn item_summary(item: &RoutineItem, tracking: Tracking) -> String {
    let reps = item.sets.iter().map(|set| set.reps.unwrap_or(0.0)).collect::<Vec<_>>();
    let unit = if tracking.timed() { "s" } else { "reps" };
    let mut summary = format!("{} de {} {unit}", count(item.sets.len(), "serie", "series"), span(&reps));
    if tracking.weighted() {
        let weights = item.sets.iter().map(|set| set.weight.unwrap_or(0.0)).collect::<Vec<_>>();
        if weights.iter().any(|weight| *weight > 0.0) {
            summary.push_str(&format!(", {} kg", span(&weights)));
        } else {
            summary.push_str(", sin peso cargado");
        }
    } else if !tracking.timed() {
        summary.push_str(", peso corporal");
    }
    summary
}

fn item_total(item: &RoutineItem, tracking: Tracking) -> String {
    let reps: f64 = item.sets.iter().map(|set| set.reps.unwrap_or(0.0)).sum();
    if tracking.weighted() && !tracking.timed() {
        format!("Volumen: {} kg", fmt(item_volume(item, tracking), 1))
    } else if tracking.timed() {
        format!("Total: {} s", fmt(reps, 1))
    } else {
        format!("Total: {} repeticiones", fmt(reps, 1))
    }
}

/// Los días con entrenamiento: calorías, minutos y series sumados; la rutina
/// es la del último del día.
#[derive(Debug, Clone, Default)]
struct DayTotal {
    kcal: f64,
    minutes: f64,
    sets: u32,
    routine: String,
    color: String,
}

fn day_totals(workouts: &[Workout]) -> BTreeMap<String, DayTotal> {
    let mut days: BTreeMap<String, DayTotal> = BTreeMap::new();
    let mut ordered = workouts.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|workout| workout.ended_ms);
    for workout in ordered {
        let day = days.entry(workout.date.clone()).or_default();
        day.kcal += workout.kcal;
        day.minutes += workout.minutes;
        day.sets += workout.sets;
        day.routine = workout.routine_name.clone();
        day.color = workout.color.clone();
    }
    days
}

/// La rutina de cada día de la semana (la primera que lo tiene).
fn assigned_days(routines: &[Routine]) -> BTreeMap<usize, &Routine> {
    let mut assigned = BTreeMap::new();
    for routine in routines {
        for day in &routine.days {
            assigned.entry(*day as usize).or_insert(routine);
        }
    }
    assigned
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

fn muscle_state(last_ms: Option<i64>, now_ms: i64) -> &'static str {
    match last_ms {
        None => "none",
        Some(last) => {
            let hours = (now_ms - last).max(0) as f64 / 3_600_000.0;
            if hours < FATIGUED_HOURS {
                "fat"
            } else if hours < RECOVERING_HOURS {
                "rec"
            } else {
                "ok"
            }
        }
    }
}

fn state_label(state: &str) -> &'static str {
    match state {
        "fat" => "Fatigado",
        "rec" => "Recuperándose",
        "ok" => "Recuperado",
        _ => "Sin trabajar",
    }
}

fn panel(data: &GymData, catalog: &Catalog, query: &GymQuery, today: Date, now_ms: i64) -> PanelView {
    let days = day_totals(&data.workouts);
    let monday = monday_of(today);
    let sunday = shift_days(monday, 6);
    let assigned = assigned_days(&data.routines);
    let target = assigned.len().max(1);
    let week_count = |start: Date| (0..7).filter(|offset| days.contains_key(&key(shift_days(start, *offset)))).count();

    let mut streak = 0;
    let mut week = shift_days(monday, -7);
    for _ in 0..80 {
        if week_count(week) < target {
            break;
        }
        streak += 1;
        week = shift_days(week, -7);
    }
    if week_count(monday) >= target {
        streak += 1;
    }
    let mut best = 0;
    if let Some(first) = days.keys().next().and_then(|date| parse_date(date)) {
        let mut run = 0;
        let mut start = monday_of(first);
        while start <= monday {
            if week_count(start) >= target {
                run += 1;
                best = best.max(run);
            } else if start < monday {
                run = 0;
            }
            start = shift_days(start, 7);
        }
    }
    best = best.max(streak);

    let week_days = (0..7).map(|offset| key(shift_days(monday, offset))).filter(|date| days.contains_key(date)).collect::<Vec<_>>();
    let sessions = week_days.len();
    let week_kcal: f64 = week_days.iter().filter_map(|date| days.get(date)).map(|day| day.kcal).sum();
    let week_minutes: f64 = week_days.iter().filter_map(|date| days.get(date)).map(|day| day.minutes).sum();
    let today_key = key(today);
    let today_index = weekday(today);
    let pending = (today_index..7)
        .filter_map(|index| assigned.get(&index).map(|routine| (index, routine)))
        .filter(|(index, _)| !days.contains_key(&key(shift_days(monday, *index as i64))))
        .map(|(_, routine)| routine.name.clone())
        .collect::<Vec<_>>();

    // Lo que toca después.
    let trained_today = days.contains_key(&today_key);
    let mut next: Option<&Routine> = None;
    let mut next_label = String::new();
    let mut next_is_today = false;
    if let (Some(routine), false) = (assigned.get(&today_index), trained_today) {
        next = Some(routine);
        next_is_today = true;
        next_label = format!("Hoy toca {}", routine.name);
    } else {
        for offset in 1..=7 {
            let index = (today_index + offset) % 7;
            if let Some(routine) = assigned.get(&index) {
                next = Some(routine);
                let rest = if trained_today { "Hoy ya entrenaste" } else { "Hoy es día de descanso" };
                next_label = format!("{rest}. Próxima: {}, el {}", routine.name, DAY_NAMES[index]);
                break;
            }
        }
    }
    if next.is_none() {
        if let Some(first) = data.routines.first() {
            next = Some(first);
            next_label = "Ninguna rutina tiene días asignados".to_string();
        } else {
            next_label = "Todavía no tenés rutinas: creá una desde Ver rutinas".to_string();
        }
    }

    // Músculos de la semana.
    let mut muscle_sets: BTreeMap<&str, f64> = BTreeMap::new();
    let mut muscle_last: BTreeMap<&str, i64> = BTreeMap::new();
    let monday_key = key(monday);
    for workout in data.workouts.iter().filter(|workout| workout.date >= monday_key && workout.date <= today_key) {
        for done in &workout.exercises {
            let Some(exercise) = catalog.exercise(&done.exercise) else { continue };
            for (muscle, _, _) in MUSCLES.iter() {
                let level = exercise.muscle_level(muscle);
                if level == 0 {
                    continue;
                }
                *muscle_sets.entry(muscle).or_default() += done.sets.len() as f64 * if level == 2 { 1.0 } else { 0.5 };
                let last = muscle_last.entry(muscle).or_insert(workout.ended_ms);
                *last = (*last).max(workout.ended_ms);
            }
        }
    }
    let mut muscle_states = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let muscles = MUSCLES
        .iter()
        .map(|(muscle, name, _)| {
            let state = muscle_state(muscle_last.get(muscle).copied(), now_ms);
            muscle_states.insert(muscle.to_string(), state.to_string());
            *counts.entry(state).or_default() += 1;
            let sets = muscle_sets.get(muscle).copied().unwrap_or(0.0);
            let detail = if sets > 0.0 { format!("{}, {} series", state_label(state), fmt(sets, 1)) } else { state_label(state).to_string() };
            MuscleRow { key: muscle.to_string(), name: name.to_string(), state: state.to_string(), detail }
        })
        .collect();
    let legend = ["fat", "rec", "ok", "none"]
        .iter()
        .map(|state| LegendRow { state: state.to_string(), label: state_label(state).to_string(), count: counts.get(state).copied().unwrap_or(0) })
        .collect();
    let advice = next
        .map(|routine| {
            let mut tired = Vec::new();
            let mut healing = Vec::new();
            for item in &routine.items {
                let Some(exercise) = catalog.exercise(&item.exercise) else { continue };
                for muscle in &exercise.primary {
                    let label = muscle_label(muscle).unwrap_or_default();
                    match muscle_states.get(muscle).map(String::as_str) {
                        Some("fat") if !tired.contains(&label) => tired.push(label),
                        Some("rec") if !healing.contains(&label) => healing.push(label),
                        _ => {}
                    }
                }
            }
            if !tired.is_empty() {
                format!(
                    "{} carga {}, que {}. Bajá un poco el peso o sumá un día de descanso.",
                    routine.name,
                    tired.join(", "),
                    if tired.len() == 1 { "todavía está fatigado" } else { "todavía están fatigados" }
                )
            } else if !healing.is_empty() {
                format!(
                    "{} carga {}, que {}. El resto de los músculos llega descansado.",
                    routine.name,
                    healing.join(", "),
                    if healing.len() == 1 { "todavía se está recuperando" } else { "todavía se están recuperando" }
                )
            } else if routine.items.is_empty() {
                format!("{} todavía no tiene ejercicios.", routine.name)
            } else {
                format!("{} trabaja músculos que ya están descansados.", routine.name)
            }
        })
        .unwrap_or_default();

    // Calendario de 24 semanas.
    let mut all_kcal = days.values().map(|day| day.kcal).collect::<Vec<_>>();
    all_kcal.sort_by(|a, b| a.total_cmp(b));
    let third = |numerator: usize| all_kcal.get(all_kcal.len() * numerator / 3).copied().unwrap_or(0.0);
    let (low, mid) = (third(1), third(2));
    let latest = days.keys().rfind(|date| **date <= today_key).cloned();
    let selected = query.day.clone().filter(|date| parse_date(date).is_some()).or(latest).unwrap_or_else(|| today_key.clone());
    let start = shift_days(monday, -7 * (HEAT_WEEKS - 1));
    let weeks = (0..HEAT_WEEKS)
        .map(|week| {
            let first = shift_days(start, 7 * week);
            let mut month = String::new();
            let days = (0..7)
                .map(|offset| {
                    let date = shift_days(first, offset);
                    if date.day() == 1 || (week == 0 && offset == 0) {
                        month = MONTHS_SHORT[date.month() as usize - 1].to_string();
                    }
                    let date_key = key(date);
                    let total = days.get(&date_key);
                    let future = date > today;
                    let level = total.map(|day| if day.kcal <= low { 1 } else if day.kcal <= mid { 2 } else { 3 }).unwrap_or(0);
                    let label = match total {
                        Some(day) => format!("{}: {}, {} kcal", long_day(date), day.routine, fmt(day.kcal, 0)),
                        None if future => format!("{}: todavía no llegó", long_day(date)),
                        None => format!("{}: sin entrenamiento", long_day(date)),
                    };
                    HeatDay {
                        date: date_key.clone(),
                        color: total.map(|day| day.color.clone()),
                        level,
                        future,
                        today: date_key == today_key,
                        selected: date_key == selected,
                        label,
                    }
                })
                .collect();
            HeatWeek { month, days }
        })
        .collect();
    let selected_date = parse_date(&selected).unwrap_or(today);
    let selected_total = days.get(&selected);
    let day = DayDetail {
        date: format!("{}{}", long_day(selected_date), if selected == today_key { " (hoy)" } else { "" }),
        has: selected_total.is_some(),
        routine: selected_total.map(|day| day.routine.clone()).unwrap_or_default(),
        color: selected_total.map(|day| day.color.clone()),
        kcal: selected_total.map(|day| fmt(day.kcal, 0)).unwrap_or_default(),
        minutes: selected_total.map(|day| fmt(day.minutes, 0)).unwrap_or_default(),
        sets: selected_total.map(|day| day.sets.to_string()).unwrap_or_default(),
        none_text: if selected_date > today { "Ese día todavía no llegó.".to_string() } else { "Día de descanso, no hay entrenamiento registrado.".to_string() },
    };

    // Calorías de los últimos 14 días.
    let bar_days = (0..BAR_DAYS).rev().map(|ago| shift_days(today, -ago)).collect::<Vec<_>>();
    let max_kcal = bar_days.iter().filter_map(|date| days.get(&key(*date))).map(|day| day.kcal).fold(1.0, f64::max);
    let mut calories_14 = 0.0;
    let bars = bar_days
        .iter()
        .map(|date| {
            let total = days.get(&key(*date));
            calories_14 += total.map(|day| day.kcal).unwrap_or(0.0);
            Bar {
                letter: DAY_LETTERS[weekday(*date)].to_string(),
                day: date.day() as u32,
                value: total.map(|day| fmt(day.kcal, 0)).unwrap_or_default(),
                height: total.map(|day| ((day.kcal / max_kcal * BAR_MAX_PX).round() as u32).max(8)).unwrap_or(3),
                color: total.map(|day| day.color.clone()),
                today: *date == today,
                label: match total {
                    Some(day) => format!("{}: {} kcal, {}", long_day(*date), fmt(day.kcal, 0), day.routine),
                    None => format!("{}: sin entrenamiento", long_day(*date)),
                },
            }
        })
        .collect();

    let stats = vec![
        StatCard {
            key: "streak".into(),
            value: count(streak, "semana", "semanas"),
            sub: format!("Con {target}+ entrenamientos por semana. Mejor: {best}."),
        },
        StatCard {
            key: "week".into(),
            value: format!("{sessions} de {target}"),
            sub: if sessions >= target {
                "Semana cumplida.".to_string()
            } else if !pending.is_empty() {
                format!("Falta: {}.", pending.join(", "))
            } else {
                "Ya no quedan días planificados.".to_string()
            },
        },
        StatCard {
            key: "kcal".into(),
            value: format!("≈ {} kcal", fmt(week_kcal, 0)),
            sub: if sessions > 0 {
                format!("Promedio de {} por entrenamiento.", fmt((week_kcal / sessions as f64).round(), 0))
            } else {
                "Todavía no entrenaste esta semana.".to_string()
            },
        },
        StatCard { key: "time".into(), value: hours_minutes(week_minutes), sub: format!("{} desde el lunes.", count(sessions, "sesión", "sesiones")) },
    ];

    PanelView {
        week_label: format!(
            "Semana del {} de {} al {} de {}",
            monday.day(),
            MONTHS[monday.month() as usize - 1],
            sunday.day(),
            MONTHS[sunday.month() as usize - 1]
        ),
        next_label,
        next_routine_id: next.map(|routine| routine.id.clone()),
        next_name: next.map(|routine| routine.name.clone()).unwrap_or_default(),
        next_is_today,
        stats,
        muscle_states,
        muscles,
        legend,
        advice,
        weeks,
        day,
        routine_legend: data.routines.iter().map(|routine| LegendColor { name: routine.name.clone(), color: routine.color.clone() }).collect(),
        bars,
        calories_14: fmt(calories_14, 0),
    }
}

// ---------------------------------------------------------------------------
// Rutinas
// ---------------------------------------------------------------------------

fn routine_rows(data: &GymData, selected: Option<&str>) -> Vec<RoutineRow> {
    data.routines
        .iter()
        .map(|routine| {
            let sets: usize = routine.items.iter().map(|item| item.sets.len()).sum();
            let mut days = routine.days.clone();
            days.sort_unstable();
            RoutineRow {
                id: routine.id.clone(),
                name: if routine.name.is_empty() { "Sin nombre".to_string() } else { routine.name.clone() },
                focus: if routine.focus.is_empty() { "Sin enfoque".to_string() } else { routine.focus.clone() },
                count: format!("{}, {}", count(routine.items.len(), "ejercicio", "ejercicios"), count(sets, "serie", "series")),
                days: if days.is_empty() {
                    "Sin día".to_string()
                } else {
                    days.iter().filter_map(|day| DAY_SHORT.get(*day as usize)).copied().collect::<Vec<_>>().join(", ")
                },
                color: routine.color.clone(),
                selected: Some(routine.id.as_str()) == selected,
            }
        })
        .collect()
}

fn train_label(data: &GymData, routine_id: &str) -> String {
    match &data.session {
        Some(session) if session.active() && session.routine_id == routine_id => "Seguir entrenando".to_string(),
        _ => "Entrenar".to_string(),
    }
}

fn routine_view(data: &GymData, catalog: &Catalog, routine: &Routine) -> RoutineView {
    let mut total_sets = 0;
    let mut total_volume = 0.0;
    let mut levels: BTreeMap<String, u8> = BTreeMap::new();
    let mut rest_total: i64 = 0;
    let items = routine
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let fallback = removed_exercise(&item.exercise);
            let exercise = exercise_of(catalog, &item.exercise, &fallback);
            total_sets += item.sets.len();
            total_volume += item_volume(item, exercise.tracking);
            for muscle in exercise.primary.iter().chain(&exercise.secondary) {
                let level = exercise.muscle_level(muscle);
                let current = levels.entry(muscle.clone()).or_default();
                *current = (*current).max(level);
            }
            rest_total += item.rest_s as i64 * item.sets.len() as i64;
            if index + 1 == routine.items.len() {
                rest_total -= item.rest_s as i64;
            }
            ItemView {
                id: item.id.clone(),
                n: index + 1,
                exercise_id: item.exercise.clone(),
                name: exercise.name.clone(),
                group: exercise.group.clone(),
                group_label: group_label(&exercise.group).to_string(),
                equipment_label: equipment_label(exercise, catalog),
                rest_s: item.rest_s,
                rest_label: rest_clock(item.rest_s as i64),
                missing: missing_text(&missing_equipment(exercise, catalog, &data.owned)),
                summary: item_summary(item, exercise.tracking),
                weighted: exercise.tracking.weighted(),
                timed: exercise.tracking.timed(),
                sets: item.sets.iter().map(|set| set_view(set, exercise.tracking)).collect(),
                total_label: item_total(item, exercise.tracking),
            }
        })
        .collect::<Vec<_>>();
    let names = |level: u8| {
        let list = MUSCLES
            .iter()
            .filter(|(muscle, _, _)| levels.get(*muscle) == Some(&level))
            .map(|(_, name, _)| *name)
            .collect::<Vec<_>>();
        if list.is_empty() {
            "Ninguno".to_string()
        } else {
            list.join(", ")
        }
    };
    RoutineView {
        id: routine.id.clone(),
        name: routine.name.clone(),
        focus: routine.focus.clone(),
        focus_label: if routine.focus.is_empty() { "Sin enfoque".to_string() } else { routine.focus.clone() },
        color: routine.color.clone(),
        days: (0..7u8).map(|day| routine.days.contains(&day)).collect(),
        stats: RoutineStats { exercises: items.len(), sets: total_sets, volume: fmt(total_volume, 1) },
        summary: MuscleSummary { primary: names(2), secondary: names(1), rest: rest_clock(rest_total.max(0)), muscles: levels },
        items,
        train_label: train_label(data, &routine.id),
    }
}

fn library(data: &GymData, catalog: &Catalog, routine: Option<&Routine>, query: &GymQuery) -> LibraryView {
    let search = fold(query.search.trim());
    let group = query.group.as_deref().filter(|group| *group != "todos");
    let in_routine = routine.map(|routine| routine.items.iter().map(|item| item.exercise.as_str()).collect::<BTreeSet<_>>()).unwrap_or_default();
    let mut hidden = 0;
    let mut rows = Vec::new();
    for exercise in &catalog.exercises {
        if group.is_some_and(|group| exercise.group != group) {
            continue;
        }
        if !search.is_empty() {
            let matches = fold(&exercise.name).contains(&search)
                || exercise.aliases.iter().any(|alias| fold(alias).contains(&search))
                || exercise.equipment.iter().filter_map(|id| catalog.equipment_name(id)).any(|name| fold(name).contains(&search));
            if !matches {
                continue;
            }
        }
        let missing = missing_equipment(exercise, catalog, &data.owned);
        if query.only_mine && !missing.is_empty() {
            hidden += 1;
            continue;
        }
        rows.push(LibraryRow {
            id: exercise.id.clone(),
            name: exercise.name.clone(),
            group: exercise.group.clone(),
            meta: format!("{}, {}", group_label(&exercise.group), lower_first(&equipment_label(exercise, catalog))),
            missing: (!missing.is_empty()).then(|| format!("Falta: {}", missing.iter().map(|name| lower_first(name)).collect::<Vec<_>>().join(", "))),
            added: in_routine.contains(exercise.id.as_str()),
        });
    }
    LibraryView { rows, hidden, hidden_note: if hidden > 0 { count(hidden, "oculto", "ocultos") } else { String::new() } }
}

// ---------------------------------------------------------------------------
// Entrenar
// ---------------------------------------------------------------------------

fn clock(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn training(data: &GymData, catalog: &Catalog, routine: &Routine) -> TrainingView {
    let session: Option<&Session> = data.session.as_ref().filter(|session| session.routine_id == routine.id);
    let done = |item: &RoutineItem, index: usize| session.is_some_and(|session| session.is_done(&item.id, index));
    let mut next: Option<(String, usize, String, usize)> = None;
    let mut done_sets = 0;
    let mut total_sets = 0;
    let mut volume = 0.0;
    let mut rates = Vec::new();
    for item in &routine.items {
        let fallback = removed_exercise(&item.exercise);
        let exercise = exercise_of(catalog, &item.exercise, &fallback);
        rates.push(if exercise.kcal_per_min > 0.0 { exercise.kcal_per_min } else { 5.0 });
        for (index, set) in item.sets.iter().enumerate() {
            total_sets += 1;
            if done(item, index) {
                done_sets += 1;
                if exercise.tracking.weighted() && !exercise.tracking.timed() {
                    volume += set.weight.unwrap_or(0.0) * set.reps.unwrap_or(0.0);
                }
            } else if next.is_none() {
                next = Some((item.id.clone(), index, exercise.name.clone(), item.sets.len()));
            }
        }
    }
    let items = routine
        .items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let fallback = removed_exercise(&item.exercise);
            let exercise = exercise_of(catalog, &item.exercise, &fallback);
            let done_count = (0..item.sets.len()).filter(|index| done(item, *index)).count();
            let current = next.as_ref().is_some_and(|(id, _, _, _)| *id == item.id);
            TrainItem {
                id: item.id.clone(),
                n: position + 1,
                exercise_id: item.exercise.clone(),
                name: exercise.name.clone(),
                group: exercise.group.clone(),
                group_label: group_label(&exercise.group).to_string(),
                rest_label: rest_clock(item.rest_s as i64),
                missing: missing_text(&missing_equipment(exercise, catalog, &data.owned)),
                weighted: exercise.tracking.weighted(),
                timed: exercise.tracking.timed(),
                count: format!("{done_count}/{}", item.sets.len()),
                all_done: done_count == item.sets.len() && !item.sets.is_empty(),
                current,
                sets: item
                    .sets
                    .iter()
                    .enumerate()
                    .map(|(index, set)| TrainSet {
                        weight: input_value(set.weight),
                        reps: input_value(set.reps),
                        done: done(item, index),
                        next: current && next.as_ref().is_some_and(|(_, next_index, _, _)| *next_index == index),
                    })
                    .collect(),
            }
        })
        .collect();
    let status = match session.map(|session| session.status) {
        Some(SessionStatus::Running) => "running",
        Some(SessionStatus::Paused) => "paused",
        Some(SessionStatus::Done) => "done",
        None => "idle",
    };
    let acc_ms = session.map(|session| session.acc_ms).unwrap_or(0);
    let current_rest = next.as_ref().and_then(|(id, _, _, _)| routine.items.iter().find(|item| &item.id == id)).map(|item| format!("{} en este ejercicio", rest_clock(item.rest_s as i64)));
    TrainingView {
        routine_id: routine.id.clone(),
        name: routine.name.clone(),
        focus_label: if routine.focus.is_empty() { "Sin enfoque".to_string() } else { routine.focus.clone() },
        status: status.to_string(),
        started_ms: session.map(|session| session.started_ms).unwrap_or(0),
        acc_ms,
        rest_end_ms: session.and_then(|session| session.rest_end_ms),
        rest_left_ms: session.and_then(|session| session.rest_left_ms),
        rest_total_ms: session.map(|session| session.rest_total_ms).unwrap_or(0),
        done_sets,
        total_sets,
        percent: if total_sets > 0 { (done_sets as f64 / total_sets as f64 * 100.0).round() as u32 } else { 0 },
        volume: format!("{} kg", fmt(volume, 1)),
        kcal_per_min: if rates.is_empty() { 0.0 } else { rates.iter().sum::<f64>() / rates.len() as f64 },
        items,
        next_text: next.map(|(_, index, name, of)| format!("serie {} de {of} de {name}", index + 1)),
        current_rest,
        done_text: format!("Hiciste {done_sets} de {total_sets} series en {}.", clock(acc_ms)),
    }
}

// ---------------------------------------------------------------------------
// Equipamiento
// ---------------------------------------------------------------------------

fn equipment_view(data: &GymData, catalog: &Catalog) -> EquipmentView {
    let uses = |id: &str| catalog.exercises.iter().filter(|exercise| exercise.equipment.iter().any(|needed| needed == id)).count();
    let order = |category: &str| EQUIPMENT_CATEGORIES.iter().position(|(key, _)| *key == category).unwrap_or(EQUIPMENT_CATEGORIES.len());
    let mut sorted = catalog.equipment.iter().collect::<Vec<&Equipment>>();
    sorted.sort_by(|a, b| a.custom.cmp(&b.custom).then(order(&a.category).cmp(&order(&b.category))).then(fold(&a.name).cmp(&fold(&b.name))));
    let items = sorted
        .iter()
        .map(|item| {
            let used = uses(&item.id);
            EquipmentRow {
                id: item.id.clone(),
                name: item.name.clone(),
                category: item.category.clone(),
                category_label: if item.custom {
                    format!("{}, agregado por vos", category_label(&item.category))
                } else {
                    category_label(&item.category).to_string()
                },
                sub: if used > 0 {
                    format!("Se usa en {}", count(used, "ejercicio", "ejercicios"))
                } else if item.custom {
                    "Asignalo desde la ficha de un ejercicio".to_string()
                } else {
                    "Ningún ejercicio lo usa todavía".to_string()
                },
                owned: data.owned.contains(&item.id),
                custom: item.custom,
                has_image: item.has_image,
            }
        })
        .collect();
    let owned_builtin = catalog.equipment.iter().filter(|item| !item.custom && data.owned.contains(&item.id)).map(|item| item.id.as_str()).collect::<BTreeSet<_>>();
    let presets = PRESETS
        .iter()
        .map(|(id, label)| {
            let set = match *id {
                "gym" => catalog.equipment.iter().filter(|item| !item.custom).map(|item| item.id.as_str()).collect::<BTreeSet<_>>(),
                "casa" => HOME_PRESET.iter().copied().filter(|id| catalog.equipment(id).is_some()).collect(),
                _ => BTreeSet::new(),
            };
            PresetChip { id: id.to_string(), label: label.to_string(), pressed: set == owned_builtin }
        })
        .collect();
    let available = catalog.exercises.iter().filter(|exercise| missing_equipment(exercise, catalog, &data.owned).is_empty()).count();
    let routines = data
        .routines
        .iter()
        .map(|routine| {
            let issues = routine
                .items
                .iter()
                .filter_map(|item| catalog.exercise(&item.exercise))
                .filter_map(|exercise| {
                    let missing = missing_equipment(exercise, catalog, &data.owned);
                    (!missing.is_empty()).then(|| Issue {
                        exercise: exercise.name.clone(),
                        need: format!("Necesita {}", missing.iter().map(|name| lower_first(name)).collect::<Vec<_>>().join(", ")),
                    })
                })
                .collect::<Vec<_>>();
            RoutineImpact {
                name: routine.name.clone(),
                color: routine.color.clone(),
                ok: issues.is_empty(),
                status: if issues.is_empty() {
                    "Podés hacerla completa".to_string()
                } else if issues.len() == 1 {
                    "1 ejercicio no se puede hacer".to_string()
                } else {
                    format!("{} ejercicios no se pueden hacer", issues.len())
                },
                issues,
            }
        })
        .collect();
    let total = catalog.exercises.len();
    EquipmentView {
        items,
        presets,
        available: format!("{available} de {total}"),
        available_percent: if total > 0 { (available as f64 / total as f64 * 100.0).round() as u32 } else { 0 },
        routines,
        categories: EQUIPMENT_CATEGORIES.iter().map(|(key, label)| Choice { key: key.to_string(), label: label.to_string() }).collect(),
    }
}

// ---------------------------------------------------------------------------
// Todo junto
// ---------------------------------------------------------------------------

/// La rutina que muestra la pantalla: la pedida, la del entrenamiento en
/// curso o la primera.
pub fn selected_routine<'a>(data: &'a GymData, query: &GymQuery) -> Option<&'a Routine> {
    query
        .routine_id
        .as_deref()
        .and_then(|id| data.routine(id))
        .or_else(|| data.session.as_ref().filter(|session| session.active()).and_then(|session| data.routine(&session.routine_id)))
        .or_else(|| data.routines.first())
}

pub fn build_view(data: &GymData, catalog: &Catalog, query: &GymQuery, today: Date, now_ms: i64) -> GymView {
    // Al abrir, un entrenamiento en curso sigue donde estaba.
    let resumed = data.session.as_ref().filter(|session| query.resume && session.active()).map(|session| GymQuery {
        screen: Screen::Entrenar,
        routine_id: Some(session.routine_id.clone()),
        resume: false,
        ..query.clone()
    });
    let query = resumed.as_ref().unwrap_or(query);
    let selected = selected_routine(data, query);
    let routine_view = selected.filter(|_| matches!(query.screen, Screen::Rutinas | Screen::Editar)).map(|routine| routine_view(data, catalog, routine));
    GymView {
        screen: query.screen,
        sex: data.sex.id().to_string(),
        sex_from_profile: data.sex_from_profile,
        routines: routine_rows(data, selected.map(|routine| routine.id.as_str())),
        routine_id: selected.map(|routine| routine.id.clone()),
        equipment_owned: catalog.equipment.iter().filter(|item| data.owned.contains(&item.id)).count(),
        equipment_total: catalog.equipment.len(),
        exercise_total: catalog.exercises.len(),
        session: data.session.as_ref().filter(|session| session.active()).map(|session| SessionBadge {
            routine_id: session.routine_id.clone(),
            routine_name: data.routine(&session.routine_id).map(|routine| routine.name.clone()).unwrap_or_default(),
            status: if session.status == SessionStatus::Paused { "paused" } else { "running" }.to_string(),
        }),
        groups: GROUPS.iter().map(|(key, label)| Choice { key: key.to_string(), label: label.to_string() }).collect(),
        panel: (query.screen == Screen::Panel).then(|| panel(data, catalog, query, today, now_ms)),
        routine: routine_view,
        library: (query.screen == Screen::Editar).then(|| library(data, catalog, selected, query)),
        training: selected.filter(|_| query.screen == Screen::Entrenar).map(|routine| training(data, catalog, routine)),
        equipment: (query.screen == Screen::Equipo).then(|| equipment_view(data, catalog)),
        now_ms,
    }
}

// ---------------------------------------------------------------------------
// Ficha
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentChip {
    pub id: String,
    pub name: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuscleChip {
    pub key: String,
    pub label: String,
    /// 2 principal, 1 secundario, 0 no lo trabaja.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseDetail {
    pub id: String,
    pub name: String,
    pub group: String,
    pub group_label: String,
    pub tracking: String,
    pub weighted: bool,
    pub timed: bool,
    pub kcal: String,
    pub image: Option<String>,
    pub has_video: bool,
    pub steps: Vec<String>,
    pub muscles: BTreeMap<String, u8>,
    pub muscle_chips: Vec<MuscleChip>,
    pub primary_text: String,
    pub secondary_text: String,
    pub meta: String,
    pub equipment: Vec<EquipmentChip>,
    pub required_text: String,
    pub status_text: String,
    pub missing: bool,
    pub projections: Vec<Projection>,
    pub path: String,
}

pub fn exercise_detail(exercise: &Exercise, catalog: &Catalog, data: &GymData) -> ExerciseDetail {
    let names = |level: u8, empty: &str| {
        let list = MUSCLES.iter().filter(|(muscle, _, _)| exercise.muscle_level(muscle) == level).map(|(_, name, _)| *name).collect::<Vec<_>>();
        if list.is_empty() {
            empty.to_string()
        } else {
            list.join(", ")
        }
    };
    let missing = missing_equipment(exercise, catalog, &data.owned);
    let required = exercise.equipment.iter().filter_map(|id| catalog.equipment_name(id)).collect::<Vec<_>>();
    let mut chips = catalog
        .equipment
        .iter()
        .map(|item| EquipmentChip { id: item.id.clone(), name: item.name.clone(), required: exercise.equipment.contains(&item.id) })
        .collect::<Vec<_>>();
    chips.sort_by(|a, b| b.required.cmp(&a.required).then(fold(&a.name).cmp(&fold(&b.name))));
    let tracking = exercise.tracking;
    ExerciseDetail {
        id: exercise.id.clone(),
        name: exercise.name.clone(),
        group: exercise.group.clone(),
        group_label: group_label(&exercise.group).to_string(),
        tracking: tracking.id().to_string(),
        weighted: tracking.weighted(),
        timed: tracking.timed(),
        kcal: fmt(exercise.kcal_per_min, 1),
        image: exercise.image.clone(),
        has_video: exercise.video.is_some(),
        steps: exercise.steps.clone(),
        muscles: MUSCLES
            .iter()
            .filter(|(muscle, _, _)| exercise.muscle_level(muscle) > 0)
            .map(|(muscle, _, _)| (muscle.to_string(), exercise.muscle_level(muscle)))
            .collect(),
        muscle_chips: MUSCLES
            .iter()
            .map(|(muscle, label, _)| MuscleChip { key: muscle.to_string(), label: label.to_string(), level: exercise.muscle_level(muscle) })
            .collect(),
        primary_text: names(2, "Sin cargar"),
        secondary_text: names(1, "Ninguno"),
        meta: format!(
            "{}, {}, {}",
            group_label(&exercise.group),
            lower_first(&equipment_label(exercise, catalog)),
            match (tracking.weighted(), tracking.timed()) {
                (true, false) => "se hace con peso",
                (false, false) => "con peso corporal",
                (true, true) => "con peso y por tiempo",
                (false, true) => "por tiempo",
            }
        ),
        equipment: chips,
        required_text: if required.is_empty() { "No necesita equipamiento".to_string() } else { required.join(", ") },
        status_text: if missing.is_empty() {
            "Tenés todo lo necesario.".to_string()
        } else {
            format!("Te falta: {}.", missing.iter().map(|name| lower_first(name)).collect::<Vec<_>>().join(", "))
        },
        missing: !missing.is_empty(),
        projections: [(10.0, "En 10 minutos"), (30.0, "En 30 minutos"), (60.0, "En 1 hora")]
            .iter()
            .map(|(minutes, label)| Projection { label: label.to_string(), value: format!("≈ {} kcal", fmt((exercise.kcal_per_min * minutes).round(), 0)) })
            .collect(),
        path: exercise.path.clone(),
    }
}
