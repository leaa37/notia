//! Gimnasio. El catálogo vive en la biblioteca como Markdown: cada
//! ejercicio en `Gym/exercises/<nombre>.md` (con su imagen embebida y, si
//! tiene, el video al lado) y cada equipamiento en `Gym/equipment/<nombre>.md`
//! (con su foto). Las rutinas, la sesión en curso, el historial de
//! entrenamientos y el equipamiento con el que cuenta cada usuario van en la
//! base de la biblioteca, por usuario. Este módulo decide qué se guarda y qué
//! se muestra; el adaptador de la app solo lee y escribe.

pub mod body;
pub mod change;
pub mod markdown;
pub mod tools;
pub mod view;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use crate::health::{fmt, fold, parse_date, parse_number};

pub const GYM_FOLDER: &str = "Gym";
pub const EXERCISES_FOLDER: &str = "Gym/exercises";
pub const EQUIPMENT_FOLDER: &str = "Gym/equipment";

/// Músculos que dibuja el cuerpo: clave, nombre y nombre en los SVG.
pub const MUSCLES: [(&str, &str, &str); 21] = [
    ("neck", "Cuello", "Neck"),
    ("upperChest", "Pecho superior", "Upper Chest"),
    ("lowerChest", "Pecho inferior", "Lower Chest"),
    ("frontDelt", "Deltoide anterior", "Front Delt"),
    ("midDelt", "Deltoide medio", "Middle Delt"),
    ("rearDelt", "Deltoide posterior", "Rear Delt"),
    ("biceps", "Bíceps", "Biceps"),
    ("triceps", "Tríceps", "Triceps"),
    ("forearms", "Antebrazos", "Forearms"),
    ("abs", "Abdominales", "Abdominals"),
    ("obliques", "Oblicuos", "Obliques"),
    ("traps", "Trapecio", "Traps"),
    ("upperBack", "Espalda alta", "Upper Back"),
    ("lats", "Dorsales", "Lats"),
    ("lowerBack", "Lumbares", "Lower Back"),
    ("glutes", "Glúteos", "Glutes"),
    ("abductors", "Abductores", "Abductors"),
    ("adductors", "Aductores", "Adductors"),
    ("quads", "Cuádriceps", "Quadriceps"),
    ("hams", "Isquiotibiales", "Hamstrings"),
    ("calves", "Pantorrillas", "Calves"),
];

/// Grupos de ejercicios, en el orden de los filtros.
pub const GROUPS: [(&str, &str); 9] = [
    ("pecho", "Pecho"),
    ("espalda", "Espalda"),
    ("piernas", "Piernas"),
    ("hombros", "Hombros"),
    ("brazos", "Brazos"),
    ("core", "Core"),
    ("completo", "Cuerpo completo"),
    ("cardio", "Cardio"),
    ("estiramiento", "Estiramiento"),
];

/// Categorías del equipamiento, en el orden de la pantalla.
pub const EQUIPMENT_CATEGORIES: [(&str, &str); 5] = [
    ("libres", "Pesos libres"),
    ("estructuras", "Bancos y estructuras"),
    ("maquinas", "Máquinas"),
    ("accesorios", "Accesorios"),
    ("otros", "Otros"),
];

/// Atajos de equipamiento: «Gimnasio completo» marca todo.
pub const PRESETS: [(&str, &str); 3] = [("gym", "Gimnasio completo"), ("casa", "Casa con mancuernas"), ("nada", "Sin equipamiento")];
pub const HOME_PRESET: [&str; 4] = ["dumbbell", "flat_bench", "pull_up_bar", "ab_wheel"];

/// Colores de las rutinas nuevas, en orden: nombres de la paleta, que el
/// tema pinta en oscuro o en claro.
pub const ROUTINE_COLORS: [&str; 7] = ["azul", "violeta", "oro", "verde", "ambar", "teal", "coral"];

pub const DEFAULT_REST_S: u32 = 90;
pub const MAX_REST_S: u32 = 600;
pub const REST_STEP_S: u32 = 15;
pub const MAX_SETS: usize = 20;
pub const MAX_ROUTINE_ITEMS: usize = 40;
pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_STEP_CHARS: usize = 600;
pub const MAX_STEPS: usize = 20;

pub fn muscle_label(key: &str) -> Option<&'static str> {
    MUSCLES.iter().find(|(id, _, _)| *id == key).map(|(_, label, _)| *label)
}

/// La clave de un músculo escrito como clave, nombre o nombre del SVG.
pub fn muscle_key(value: &str) -> Option<&'static str> {
    let folded = fold(value.trim());
    MUSCLES
        .iter()
        .find(|(key, label, svg)| fold(key) == folded || fold(label) == folded || fold(svg) == folded)
        .map(|(key, _, _)| *key)
}

pub fn group_label(key: &str) -> &'static str {
    GROUPS.iter().find(|(id, _)| *id == key).map(|(_, label)| *label).unwrap_or("Otros")
}

pub fn is_group(key: &str) -> bool {
    GROUPS.iter().any(|(id, _)| *id == key)
}

pub fn category_label(key: &str) -> &'static str {
    EQUIPMENT_CATEGORIES.iter().find(|(id, _)| *id == key).map(|(_, label)| *label).unwrap_or("Otros")
}

pub fn is_category(key: &str) -> bool {
    EQUIPMENT_CATEGORIES.iter().any(|(id, _)| *id == key)
}

// ---------------------------------------------------------------------------
// Catálogo (archivos de la biblioteca)
// ---------------------------------------------------------------------------

/// Qué se anota en cada serie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Tracking {
    /// Peso y repeticiones.
    #[default]
    WeightReps,
    /// Repeticiones con el peso del cuerpo.
    Reps,
    /// Segundos.
    Time,
    /// Peso y segundos (cargar y caminar).
    WeightTime,
}

impl Tracking {
    pub fn id(self) -> &'static str {
        match self {
            Self::WeightReps => "peso-reps",
            Self::Reps => "reps",
            Self::Time => "tiempo",
            Self::WeightTime => "peso-tiempo",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match fold(value.trim()).as_str() {
            "peso-reps" => Some(Self::WeightReps),
            "reps" => Some(Self::Reps),
            "tiempo" => Some(Self::Time),
            "peso-tiempo" => Some(Self::WeightTime),
            _ => None,
        }
    }

    pub fn weighted(self) -> bool {
        matches!(self, Self::WeightReps | Self::WeightTime)
    }

    pub fn timed(self) -> bool {
        matches!(self, Self::Time | Self::WeightTime)
    }

    /// El mismo registro con o sin peso.
    pub fn with_weight(self, weighted: bool) -> Self {
        match (self.timed(), weighted) {
            (false, true) => Self::WeightReps,
            (false, false) => Self::Reps,
            (true, true) => Self::WeightTime,
            (true, false) => Self::Time,
        }
    }
}

/// Un ejercicio del catálogo. `image` solo se carga para la ficha: las
/// listas usan `has_image`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Exercise {
    pub id: String,
    pub name: String,
    pub group: String,
    pub tracking: Tracking,
    pub kcal_per_min: f64,
    /// Claves de `MUSCLES`.
    pub primary: Vec<String>,
    pub secondary: Vec<String>,
    /// Ids del equipamiento que necesita.
    pub equipment: Vec<String>,
    pub steps: Vec<String>,
    /// Nombre del archivo de video, al lado del `.md`.
    pub video: Option<String>,
    pub aliases: Vec<String>,
    /// Ruta lógica del `.md`.
    pub path: String,
    pub has_image: bool,
    /// `data:` de la imagen, cuando se leyó entera.
    pub image: Option<String>,
    /// Líneas del frontmatter que no son del ejercicio (las de toda nota),
    /// que se conservan al reescribirlo.
    pub extra_front: Vec<(String, String)>,
}

impl Exercise {
    /// 2 = principal, 1 = secundario, 0 = no lo trabaja.
    pub fn muscle_level(&self, key: &str) -> u8 {
        if self.primary.iter().any(|muscle| muscle == key) {
            2
        } else if self.secondary.iter().any(|muscle| muscle == key) {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Equipment {
    pub id: String,
    pub name: String,
    pub category: String,
    /// Agregado por la persona (se puede editar y quitar).
    pub custom: bool,
    pub path: String,
    pub has_image: bool,
    pub image: Option<String>,
    pub extra_front: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Catalog {
    pub exercises: Vec<Exercise>,
    pub equipment: Vec<Equipment>,
}

impl Catalog {
    pub fn exercise(&self, id: &str) -> Option<&Exercise> {
        self.exercises.iter().find(|exercise| exercise.id == id)
    }

    pub fn equipment(&self, id: &str) -> Option<&Equipment> {
        self.equipment.iter().find(|item| item.id == id)
    }

    /// Nombre del equipamiento, si existe.
    pub fn equipment_name(&self, id: &str) -> Option<&str> {
        self.equipment(id).map(|item| item.name.as_str())
    }
}

// ---------------------------------------------------------------------------
// Datos de cada usuario (base de la biblioteca)
// ---------------------------------------------------------------------------

/// Una serie planificada o hecha: peso en kg y repeticiones (o segundos).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetPlan {
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub reps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineItem {
    pub id: String,
    pub exercise: String,
    pub rest_s: u32,
    pub sets: Vec<SetPlan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub focus: String,
    /// 0 = lunes … 6 = domingo.
    #[serde(default)]
    pub days: Vec<u8>,
    pub color: String,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub items: Vec<RoutineItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkoutExercise {
    pub exercise: String,
    pub sets: Vec<SetPlan>,
}

/// Un entrenamiento terminado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workout {
    pub id: String,
    /// Fecha local `AAAA-MM-DD`.
    pub date: String,
    pub routine_id: String,
    pub routine_name: String,
    pub color: String,
    pub ended_ms: i64,
    pub minutes: f64,
    pub kcal: f64,
    pub sets: u32,
    pub volume: f64,
    #[serde(default)]
    pub exercises: Vec<WorkoutExercise>,
    /// El entrenamiento del reloj (Health Connect) que lo completó o lo creó.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch_id: Option<String>,
    /// Pulso promedio y máximo del reloj durante el entrenamiento.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpm: Option<f64>,
}

/// El nombre en Gimnasio de un tipo de entrenamiento del reloj (la clave de
/// Health Connect), o `None` si no se suma: caminar ya cuenta en los pasos
/// y el reloj detecta caminatas solo.
pub fn watch_session_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "strength_training" | "weightlifting" | "calisthenics" => "Fuerza",
        "running" | "running_treadmill" => "Correr",
        "biking" | "biking_stationary" => "Bici",
        "swimming_pool" | "swimming_open_water" => "Natación",
        "elliptical" => "Elíptico",
        "rowing_machine" => "Remo",
        "high_intensity_interval_training" => "HIIT",
        "stair_climbing" => "Escaleras",
        "hiking" => "Trekking",
        "yoga" => "Yoga",
        "pilates" => "Pilates",
        "other_workout" => "Entrenamiento",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    Running,
    Paused,
    Done,
}

/// El entrenamiento en curso de un usuario. Los tiempos son milisegundos
/// Unix; la pantalla cuenta el reloj con ellos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub routine_id: String,
    pub status: SessionStatus,
    /// Inicio del tramo que corre ahora.
    pub started_ms: i64,
    /// Tiempo acumulado antes de ese tramo.
    pub acc_ms: i64,
    #[serde(default)]
    pub rest_end_ms: Option<i64>,
    #[serde(default)]
    pub rest_left_ms: Option<i64>,
    #[serde(default)]
    pub rest_total_ms: i64,
    /// Series hechas por ítem de la rutina.
    #[serde(default)]
    pub done: BTreeMap<String, Vec<bool>>,
}

impl Session {
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        self.acc_ms + if self.status == SessionStatus::Running { (now_ms - self.started_ms).max(0) } else { 0 }
    }

    pub fn is_done(&self, item_id: &str, index: usize) -> bool {
        self.done.get(item_id).and_then(|sets| sets.get(index)).copied().unwrap_or(false)
    }

    pub fn active(&self) -> bool {
        matches!(self.status, SessionStatus::Running | SessionStatus::Paused)
    }

    /// Un entrenamiento en curso con actividad reciente: al abrir la app se
    /// vuelve a él. Uno olvidado hace horas no se impone al abrir (sigue
    /// en Gimnasio). `started_ms` es el comienzo del tramo que corre o el
    /// momento de la pausa.
    pub fn resumable(&self, now_ms: i64) -> bool {
        self.active() && now_ms - self.started_ms < RESUME_WINDOW_MS
    }
}

/// Cuánto después del último tramo un entrenamiento se retoma solo al abrir la app.
pub const RESUME_WINDOW_MS: i64 = 6 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodySex {
    #[default]
    Male,
    Female,
}

impl BodySex {
    pub fn id(self) -> &'static str {
        match self {
            Self::Male => "masculino",
            Self::Female => "femenino",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GymData {
    pub routines: Vec<Routine>,
    pub workouts: Vec<Workout>,
    /// Equipamiento con el que cuenta.
    pub owned: BTreeSet<String>,
    pub session: Option<Session>,
    pub sex: BodySex,
    /// Si el sexo viene del perfil de Salud.
    pub sex_from_profile: bool,
    /// El pulso dormido de cada noche, de Salud (del reloj).
    pub sleep_bpm: BTreeMap<String, f64>,
}

impl GymData {
    pub fn routine(&self, id: &str) -> Option<&Routine> {
        self.routines.iter().find(|routine| routine.id == id)
    }
}

// ---------------------------------------------------------------------------
// Errores
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GymErrorCode {
    Validation,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GymError {
    pub code: GymErrorCode,
    pub message: String,
}

impl GymError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self { code: GymErrorCode::Validation, message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self { code: GymErrorCode::NotFound, message: message.into() }
    }
}

pub type GymResult<T> = Result<T, GymError>;

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

/// «1:30» para un descanso en segundos.
pub fn rest_clock(seconds: i64) -> String {
    let seconds = seconds.max(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// «4 h 5 min» o «45 min».
pub fn hours_minutes(minutes: f64) -> String {
    let total = minutes.max(0.0).round() as i64;
    if total >= 60 {
        format!("{} h {} min", total / 60, total % 60)
    } else {
        format!("{total} min")
    }
}

/// «10» o «8–12»: el rango de unos valores.
pub fn span(values: &[f64]) -> String {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() {
        return "0".to_string();
    }
    if (max - min).abs() < f64::EPSILON {
        fmt(min, 1)
    } else {
        format!("{}–{}", fmt(min, 1), fmt(max, 1))
    }
}

/// Plural simple: «1 serie», «3 series».
pub fn count(value: usize, one: &str, many: &str) -> String {
    format!("{value} {}", if value == 1 { one } else { many })
}

/// Primera letra en minúscula, para listas dentro de una frase.
pub fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Un nombre de archivo seguro a partir de un nombre visible.
pub fn file_stem(name: &str) -> String {
    let cleaned = name
        .chars()
        .map(|character| if matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { character })
        .collect::<String>();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = trimmed.trim_matches('.').trim();
    if trimmed.is_empty() {
        "Sin nombre".to_string()
    } else {
        trimmed.chars().take(MAX_NAME_CHARS).collect()
    }
}

/// Videos de demostración de una ficha, en orden de preferencia.
pub const VIDEO_EXTENSIONS: [&str; 3] = ["mp4", "webm", "mov"];

/// La extensión de video de un nombre de archivo (`mp4`, `webm` o `mov`).
fn video_extension_of(name: &str) -> Option<&'static str> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    VIDEO_EXTENSIONS.iter().copied().find(|known| extension.eq_ignore_ascii_case(known))
}

/// Si `name` es un video de demostración.
pub fn is_video_name(name: &str) -> bool {
    video_extension_of(name).is_some()
}

/// Si el video se llama como el id del ejercicio (`<id>.mp4`, sin importar
/// mayúsculas): ese se encuentra solo y no necesita `video:` en la ficha.
pub fn is_video_of_id(id: &str, name: &str) -> bool {
    !id.is_empty() && is_video_name(name) && name.rsplit_once('.').is_some_and(|(stem, _)| stem.eq_ignore_ascii_case(id))
}

/// El video de un ejercicio que no lo nombra en su ficha: el archivo de su
/// carpeta que se llama como su id (`<id>.mp4`; si no hay, `.webm` o `.mov`).
/// `names` son los nombres de los archivos de esa carpeta.
pub fn video_by_id<'a>(id: &str, names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let candidates = names.into_iter().filter(|name| is_video_of_id(id, name)).collect::<Vec<_>>();
    VIDEO_EXTENSIONS
        .iter()
        .find_map(|extension| candidates.iter().find(|name| video_extension_of(name) == Some(*extension)))
        .map(|name| name.to_string())
}

/// Un id estable a partir de un nombre: minúsculas, sin acentos, con `_`.
pub fn slug_id(name: &str) -> String {
    let folded = fold(name);
    let mut id = String::new();
    for character in folded.chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character);
        } else if !id.ends_with('_') && !id.is_empty() {
            id.push('_');
        }
    }
    let id = id.trim_end_matches('_').to_string();
    if id.is_empty() {
        "item".to_string()
    } else {
        id
    }
}
