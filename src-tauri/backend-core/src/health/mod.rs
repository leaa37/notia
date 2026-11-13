//! Salud: perfil, peso, objetivo y plan de calorías, alimentación del día,
//! agua y composición corporal medida con una balanza. Todo por usuario de
//! la biblioteca. Este módulo es puro: calcula, valida y arma lo que ve la
//! pantalla; la app guarda los datos y llama a la IA.

pub mod ai;
pub mod change;
pub mod dashboard;
pub mod reference;
pub mod tools;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Kilocalorías de un kilo de peso corporal, para el ritmo del plan.
pub const KCAL_PER_KG: f64 = 7700.0;
/// Ritmos de cambio de peso que ofrece el objetivo, en kg por semana.
pub const PACES: [f64; 4] = [0.25, 0.5, 0.75, 1.0];
pub const DEFAULT_PACE: f64 = 0.5;
/// Pesos aceptados, en kg.
pub const MIN_WEIGHT_KG: f64 = 20.0;
pub const MAX_WEIGHT_KG: f64 = 400.0;
const MAX_MEAL_NAME_CHARS: usize = 120;
const MAX_WATER_ML: i64 = 20_000;
/// Pasos de un día como máximo.
pub const MAX_STEPS: i64 = 150_000;
/// Meta orientativa de pasos por día.
pub const STEPS_GOAL: i64 = 8_000;
/// Sueño recomendado para adultos, en minutos (7 a 9 h).
pub const SLEEP_RANGE_MINUTES: (i64, i64) = (7 * 60, 9 * 60);
const MAX_SLEEP_MINUTES: i64 = 24 * 60;

/// Factores de actividad (Harris-Benedict) con su descripción.
pub const ACTIVITIES: [(f64, &str); 5] = [
    (1.2, "Sedentario (poco o nada de ejercicio)"),
    (1.375, "Ligero (1 a 3 días por semana)"),
    (1.55, "Moderado (3 a 5 días por semana)"),
    (1.725, "Alto (6 a 7 días por semana)"),
    (1.9, "Muy alto (trabajo físico o doble turno)"),
];
pub const DEFAULT_ACTIVITY: f64 = 1.375;

pub fn activity_label(factor: f64) -> Option<&'static str> {
    ACTIVITIES.iter().find(|(value, _)| (value - factor).abs() < 1e-6).map(|(_, label)| *label)
}

// ---------------------------------------------------------------------------
// Modelo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sex {
    #[serde(rename = "M")]
    Male,
    #[serde(rename = "F")]
    Female,
}

impl Sex {
    pub fn parse(value: &str) -> Option<Self> {
        match fold(value).trim() {
            "m" | "masculino" | "hombre" | "varon" | "male" => Some(Self::Male),
            "f" | "femenino" | "mujer" | "female" => Some(Self::Female),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Male => "M",
            Self::Female => "F",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Male => "Masculino",
            Self::Female => "Femenino",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// `YYYY-MM-DD`.
    pub birth_date: String,
    pub sex: Sex,
    pub height_cm: f64,
    pub activity: f64,
    /// Meta diaria de agua que eligió la persona; sin ella se calcula.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_goal_ml: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightEntry {
    pub date: String,
    pub kg: f64,
}

/// Una medición de la balanza: el peso y los valores que informó, por clave
/// de [`METRICS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub date: String,
    pub weight: f64,
    pub values: BTreeMap<String, f64>,
}

impl Measurement {
    pub fn value(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Objective {
    pub target_kg: Option<f64>,
    pub pace: f64,
}

impl Default for Objective {
    fn default() -> Self {
        Self { target_kg: None, pace: DEFAULT_PACE }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanSource {
    Calculated,
    Ai,
}

/// Calorías y macros diarios. Sin plan guardado se usa el de mantenimiento.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
    pub weeks: i64,
    pub summary: String,
    #[serde(default)]
    pub recommendations: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub source: PlanSource,
    pub generated_on: String,
    /// Mínimo de calorías: el metabolismo basal, y nunca menos de 1200 o 1500.
    pub floor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MealCategory {
    Desayuno,
    Snack,
    Almuerzo,
    Merienda,
    Cena,
}

impl MealCategory {
    pub const ALL: [Self; 5] = [Self::Desayuno, Self::Snack, Self::Almuerzo, Self::Merienda, Self::Cena];

    pub fn id(self) -> &'static str {
        match self {
            Self::Desayuno => "desayuno",
            Self::Snack => "snack",
            Self::Almuerzo => "almuerzo",
            Self::Merienda => "merienda",
            Self::Cena => "cena",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Desayuno => "Desayuno",
            Self::Snack => "Snack",
            Self::Almuerzo => "Almuerzo",
            Self::Merienda => "Merienda",
            Self::Cena => "Cena",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match fold(value).trim() {
            "desayuno" | "breakfast" => Some(Self::Desayuno),
            "snack" | "colacion" | "tentempie" => Some(Self::Snack),
            "almuerzo" | "lunch" | "comida" => Some(Self::Almuerzo),
            "merienda" | "te" => Some(Self::Merienda),
            "cena" | "dinner" => Some(Self::Cena),
            _ => None,
        }
    }

    /// La categoría que corresponde a una hora del día (0 a 23).
    pub fn for_hour(hour: u32) -> Self {
        match hour {
            0..=10 => Self::Desayuno,
            11 => Self::Snack,
            12..=14 => Self::Almuerzo,
            15 => Self::Snack,
            16..=19 => Self::Merienda,
            _ => Self::Cena,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nutrition {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
}

impl Nutrition {
    pub fn add(self, other: Self) -> Self {
        Self {
            kcal: self.kcal + other.kcal,
            protein_g: self.protein_g + other.protein_g,
            carbs_g: self.carbs_g + other.carbs_g,
            fat_g: self.fat_g + other.fat_g,
            fiber_g: self.fiber_g + other.fiber_g,
        }
    }

    pub fn get(&self, key: MacroKey) -> f64 {
        match key {
            MacroKey::Protein => self.protein_g,
            MacroKey::Carbs => self.carbs_g,
            MacroKey::Fat => self.fat_g,
            MacroKey::Fiber => self.fiber_g,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meal {
    pub id: String,
    pub date: String,
    pub category: MealCategory,
    pub name: String,
    pub nutrition: Nutrition,
    pub created_at_ms: i64,
    /// The recipe of Recetas the meal was logged from, if any.
    #[serde(default)]
    pub recipe_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroKey {
    Protein,
    Carbs,
    Fat,
    Fiber,
}

impl MacroKey {
    pub const ALL: [Self; 4] = [Self::Protein, Self::Carbs, Self::Fat, Self::Fiber];

    pub fn id(self) -> &'static str {
        match self {
            Self::Protein => "proteinaG",
            Self::Carbs => "carbosG",
            Self::Fat => "grasasG",
            Self::Fiber => "fibraG",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Protein => "Proteínas",
            Self::Carbs => "Carbohidratos",
            Self::Fat => "Grasas",
            Self::Fiber => "Fibra",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Protein => "Prot",
            Self::Carbs => "Carb",
            Self::Fat => "Grasa",
            Self::Fiber => "Fibra",
        }
    }

    pub fn of_plan(self, plan: &Plan) -> f64 {
        match self {
            Self::Protein => plan.protein_g,
            Self::Carbs => plan.carbs_g,
            Self::Fat => plan.fat_g,
            Self::Fiber => plan.fiber_g,
        }
    }
}

/// Todo lo guardado de un usuario.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HealthData {
    pub profile: Option<Profile>,
    /// Ordenados por fecha.
    pub weights: Vec<WeightEntry>,
    /// Ordenadas por fecha.
    pub measurements: Vec<Measurement>,
    pub objective: Objective,
    pub plan: Option<Plan>,
    /// Mililitros por fecha.
    pub water: BTreeMap<String, i64>,
    /// Pasos por fecha.
    pub steps: BTreeMap<String, i64>,
    /// Minutos de sueño por fecha: la del día en que se despertó.
    pub sleep: BTreeMap<String, i64>,
    pub meals: Vec<Meal>,
}

// ---------------------------------------------------------------------------
// Métricas de la balanza
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricGroup {
    Fat,
    Muscle,
    Water,
    General,
}

impl MetricGroup {
    pub const ALL: [Self; 4] = [Self::Fat, Self::Muscle, Self::Water, Self::General];

    pub fn id(self) -> &'static str {
        match self {
            Self::Fat => "grasa",
            Self::Muscle => "musculo",
            Self::Water => "agua",
            Self::General => "general",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Fat => "Grasa",
            Self::Muscle => "Músculo",
            Self::Water => "Agua y huesos",
            Self::General => "General",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MetricDef {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub decimals: u8,
    pub group: MetricGroup,
    pub min: f64,
    pub max: f64,
}

const fn metric(key: &'static str, label: &'static str, unit: &'static str, decimals: u8, group: MetricGroup, min: f64, max: f64) -> MetricDef {
    MetricDef { key, label, unit, decimals, group, min, max }
}

pub const METRICS: [MetricDef; 16] = [
    metric("grasaPct", "Grasa corporal", "%", 1, MetricGroup::Fat, 1.0, 80.0),
    metric("grasaKg", "Peso de grasa corporal", "kg", 1, MetricGroup::Fat, 0.0, 300.0),
    metric("grasaVisceral", "Grasa visceral", "nivel", 0, MetricGroup::Fat, 1.0, 60.0),
    metric("obesidadPct", "Grado de obesidad", "%", 1, MetricGroup::Fat, -80.0, 400.0),
    metric("mmePct", "Masa muscular esquelética", "%", 1, MetricGroup::Muscle, 1.0, 80.0),
    metric("mmeKg", "Masa muscular esquelética", "kg", 1, MetricGroup::Muscle, 1.0, 200.0),
    metric("musculoPct", "Músculo", "%", 1, MetricGroup::Muscle, 1.0, 95.0),
    metric("musculoKg", "Peso muscular", "kg", 1, MetricGroup::Muscle, 1.0, 250.0),
    metric("proteinaPct", "Proteína", "%", 1, MetricGroup::Muscle, 1.0, 50.0),
    metric("aguaPct", "Agua", "%", 1, MetricGroup::Water, 10.0, 90.0),
    metric("aguaKg", "Peso del agua", "kg", 1, MetricGroup::Water, 1.0, 250.0),
    metric("huesosKg", "Huesos", "kg", 1, MetricGroup::Water, 0.5, 20.0),
    metric("imc", "IMC", "", 1, MetricGroup::General, 8.0, 90.0),
    metric("pesoSinGrasa", "Peso sin grasa", "kg", 1, MetricGroup::General, 10.0, 300.0),
    metric("metabolismo", "Metabolismo basal", "kcal", 0, MetricGroup::General, 500.0, 6000.0),
    metric("edadMetabolica", "Edad metabólica", "años", 0, MetricGroup::General, 10.0, 120.0),
];

pub fn metric_def(key: &str) -> Option<&'static MetricDef> {
    METRICS.iter().find(|metric| metric.key == key)
}

// ---------------------------------------------------------------------------
// Fechas
// ---------------------------------------------------------------------------

pub fn parse_date(value: &str) -> Option<Date> {
    value.trim().parse::<Date>().ok()
}

pub fn days_ago(today: Date, days: i64) -> Date {
    today.checked_sub(jiff::Span::new().days(days)).unwrap_or(today)
}

pub fn shift_days(date: Date, days: i64) -> Date {
    date.checked_add(jiff::Span::new().days(days)).unwrap_or(date)
}

const WEEKDAYS: [&str; 7] = ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
const WEEKDAY_NARROW: [&str; 7] = ["L", "M", "X", "J", "V", "S", "D"];
const MONTHS_SHORT: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
const MONTHS_LONG: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];

fn weekday_index(date: Date) -> usize {
    date.weekday().to_monday_zero_offset() as usize
}

pub fn weekday_narrow(date: Date) -> &'static str {
    WEEKDAY_NARROW[weekday_index(date)]
}

/// «26 sept 2026».
pub fn short_date(date: Date) -> String {
    format!("{} {} {}", date.day(), MONTHS_SHORT[date.month() as usize - 1], date.year())
}

/// «26 de septiembre de 2026».
pub fn long_date(date: Date) -> String {
    format!("{} de {} de {}", date.day(), MONTHS_LONG[date.month() as usize - 1], date.year())
}

/// «Hoy», «Ayer» o «lunes 21 sept».
pub fn day_label(date: Date, today: Date) -> String {
    if date == today {
        "Hoy".to_string()
    } else if date == days_ago(today, 1) {
        "Ayer".to_string()
    } else {
        format!("{} {} {}", WEEKDAYS[weekday_index(date)], date.day(), MONTHS_SHORT[date.month() as usize - 1])
    }
}

/// «26/09».
pub fn day_month(date: Date) -> String {
    format!("{:02}/{:02}", date.day(), date.month())
}

// ---------------------------------------------------------------------------
// Números
// ---------------------------------------------------------------------------

/// Un número como lo escribe es-AR, con hasta `decimals` decimales:
/// «1.150», «82,4», «0,25».
pub fn fmt(value: f64, decimals: u8) -> String {
    let factor = 10f64.powi(decimals as i32);
    let rounded = (value * factor).round() / factor;
    let negative = rounded < 0.0;
    let absolute = rounded.abs();
    let whole = absolute.trunc() as u64;
    let mut fraction = format!("{:.*}", decimals as usize, absolute - absolute.trunc());
    // «0.25» → «25»; los ceros del final no se escriben.
    fraction = fraction.split_once('.').map(|(_, digits)| digits.trim_end_matches('0').to_string()).unwrap_or_default();
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    let sign = if negative && (whole > 0 || !fraction.is_empty()) { "-" } else { "" };
    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped},{fraction}")
    }
}

/// Con signo: «+1,2», «-3,6», «0».
pub fn signed(value: f64, decimals: u8) -> String {
    let text = fmt(value, decimals);
    if value > 0.0 && text != "0" {
        format!("+{text}")
    } else {
        text
    }
}

pub fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Un número escrito a mano: «82,4», «82.4», «1.150» (miles), «70 kg».
pub fn parse_number(value: &str) -> Option<f64> {
    let cleaned: String = value.trim().chars().filter(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-')).collect();
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    let normalized = if cleaned.contains(',') {
        cleaned.replace('.', "").replace(',', ".")
    } else if cleaned.matches('.').count() == 1 && cleaned.split('.').nth(1).is_some_and(|tail| tail.len() == 3) && cleaned.len() > 4 {
        cleaned.replace('.', "")
    } else {
        cleaned
    };
    normalized.parse::<f64>().ok().filter(|number| number.is_finite())
}

/// Texto para comparar: minúsculas y sin acentos.
pub fn fold(value: &str) -> String {
    crate::recipes::fold(value)
}

// ---------------------------------------------------------------------------
// Cálculos
// ---------------------------------------------------------------------------

pub fn age(birth_date: &str, today: Date) -> Option<i64> {
    let birth = parse_date(birth_date)?;
    let mut years = i64::from(today.year()) - i64::from(birth.year());
    if (today.month(), today.day()) < (birth.month(), birth.day()) {
        years -= 1;
    }
    Some(years)
}

pub fn bmi(kg: f64, height_cm: f64) -> Option<f64> {
    (kg > 0.0 && height_cm > 0.0).then(|| kg / (height_cm / 100.0).powi(2))
}

/// Mifflin-St Jeor: solo si la balanza no informa el metabolismo basal.
pub fn bmr_mifflin(kg: f64, height_cm: f64, age: i64, sex: Sex) -> f64 {
    10.0 * kg + 6.25 * height_cm - 5.0 * age as f64 + if sex == Sex::Female { -161.0 } else { 5.0 }
}

/// Agua diaria: superficie corporal (Mosteller) × 1500 ml/m², de a 50 ml.
pub fn water_goal_ml(kg: f64, height_cm: f64) -> i64 {
    let surface = ((height_cm * kg) / 3600.0).sqrt();
    ((surface * 1500.0 / 50.0).round() * 50.0) as i64
}

pub fn kcal_from_macros(protein_g: f64, carbs_g: f64, fat_g: f64) -> f64 {
    (4.0 * protein_g + 4.0 * carbs_g + 9.0 * fat_g).round()
}

/// Mínimo de calorías diarias: el metabolismo basal, y nunca menos de 1200
/// (mujeres) o 1500 (hombres).
pub fn calorie_floor(bmr: f64, sex: Sex) -> f64 {
    bmr.round().max(if sex == Sex::Female { 1200.0 } else { 1500.0 })
}

#[derive(Debug, Clone, Copy)]
pub struct PlanInput {
    pub current_kg: f64,
    pub target_kg: f64,
    pub pace: f64,
    pub tdee: f64,
    pub bmr: f64,
    pub sex: Sex,
}

/// El plan sin IA: déficit o superávit según el ritmo, con el ritmo limitado
/// al 1 % del peso por semana, el ajuste a 500 kcal y el piso de calorías.
pub fn calculate_plan(input: PlanInput, today: Date) -> Plan {
    let floor = calorie_floor(input.bmr, input.sex);
    let difference = input.target_kg - input.current_kg;
    let mut notes = Vec::new();

    let mut pace = input.pace;
    let max_pace = ((input.current_kg * 0.01) * 100.0).round() / 100.0;
    if pace > max_pace {
        pace = max_pace;
        notes.push(format!("El ritmo se ajustó a {} kg por semana (1 % de tu peso).", fmt(max_pace, 2)));
    }

    let mut adjustment = 0.0;
    if difference.abs() >= 0.5 {
        adjustment = ((pace * KCAL_PER_KG) / 7.0).round() * difference.signum();
    }
    if adjustment > 500.0 {
        adjustment = 500.0;
    }

    let mut kcal = (input.tdee + adjustment).round();
    if kcal < floor {
        notes.push(format!("Las calorías se ajustaron a {} kcal para no quedar por debajo de tu metabolismo basal.", fmt(floor, 0)));
        kcal = floor;
    }

    let real_pace = ((input.tdee - kcal).abs() * 7.0) / KCAL_PER_KG;
    let weeks = if difference.abs() < 0.5 { 0 } else { (difference.abs() / real_pace.max(0.05)).ceil() as i64 };
    let reference_kg = if difference < 0.0 { input.target_kg } else { input.current_kg };
    let protein_g = (reference_kg * 1.8).round();
    let fat_g = ((kcal * 0.27) / 9.0).round();
    let fiber_g = ((kcal / 1000.0) * 14.0).round();
    let carbs_g = ((kcal - protein_g * 4.0 - fat_g * 9.0) / 4.0).round().max(0.0);
    let summary = if difference.abs() < 0.5 {
        "Plan de mantenimiento: estás en tu peso objetivo.".to_string()
    } else {
        format!(
            "{} de {} kcal por día sobre tu gasto estimado.",
            if difference < 0.0 { "Déficit" } else { "Superávit" },
            fmt((input.tdee - kcal).abs(), 0)
        )
    };
    Plan {
        kcal,
        protein_g,
        carbs_g,
        fat_g,
        fiber_g,
        weeks,
        summary,
        recommendations: Vec::new(),
        notes,
        source: PlanSource::Calculated,
        generated_on: today.to_string(),
        floor,
    }
}

/// Completa lo que se deriva de una medición si la balanza no lo trajo.
pub fn complete_measurement(measurement: &Measurement, height_cm: Option<f64>) -> Measurement {
    let mut result = measurement.clone();
    let weight = result.weight;
    let mut fill = |key: &str, value: Option<f64>| {
        if result.values.get(key).is_none() {
            if let Some(value) = value {
                result.values.insert(key.to_string(), round1(value));
            }
        }
    };
    fill("imc", height_cm.and_then(|height| bmi(weight, height)));
    let fat_pct = measurement.value("grasaPct");
    fill("grasaKg", fat_pct.map(|pct| weight * pct / 100.0));
    let fat_kg = result.values.get("grasaKg").copied();
    let mut fill = |key: &str, value: Option<f64>| {
        if result.values.get(key).is_none() {
            if let Some(value) = value {
                result.values.insert(key.to_string(), round1(value));
            }
        }
    };
    fill("pesoSinGrasa", fat_kg.map(|kg| weight - kg));
    fill("aguaKg", measurement.value("aguaPct").map(|pct| weight * pct / 100.0));
    fill("musculoKg", measurement.value("musculoPct").map(|pct| weight * pct / 100.0));
    fill("mmeKg", measurement.value("mmePct").map(|pct| weight * pct / 100.0));
    result
}

/// Lo que se calcula del perfil y los registros.
#[derive(Debug, Clone, PartialEq)]
pub struct Vitals {
    pub age: Option<i64>,
    pub current_kg: Option<f64>,
    pub bmi: Option<f64>,
    pub bmr: Option<f64>,
    pub bmr_from_scale: bool,
    pub tdee: Option<f64>,
    /// La meta de agua que rige: la elegida en el perfil o la calculada.
    pub water_goal_ml: Option<i64>,
    /// La calculada por altura y peso, aunque rija la elegida.
    pub water_goal_auto_ml: Option<i64>,
    /// Plan de mantenimiento, el objetivo cuando no hay plan guardado.
    pub maintenance: Option<Plan>,
}

pub fn vitals(data: &HealthData, today: Date) -> Vitals {
    let profile = data.profile.as_ref();
    let age = profile.and_then(|profile| age(&profile.birth_date, today));
    let last_measurement = data.measurements.last();
    let current_kg = data.weights.last().map(|entry| entry.kg).or(last_measurement.map(|m| m.weight));
    let bmi = profile.zip(current_kg).and_then(|(profile, kg)| bmi(kg, profile.height_cm));
    let scale_bmr = last_measurement.and_then(|m| m.value("metabolismo")).filter(|value| *value > 0.0);
    let bmr = scale_bmr.or_else(|| {
        let profile = profile?;
        Some(bmr_mifflin(current_kg?, profile.height_cm, age?, profile.sex))
    });
    let tdee = profile.zip(bmr).map(|(profile, bmr)| (bmr * profile.activity).round());
    let water_goal_auto_ml = profile.zip(current_kg).map(|(profile, kg)| water_goal_ml(kg, profile.height_cm));
    let water_goal_ml = profile.and_then(|profile| profile.water_goal_ml).or(water_goal_auto_ml);
    let maintenance = match (profile, current_kg, tdee, bmr) {
        (Some(profile), Some(kg), Some(tdee), Some(bmr)) => Some(calculate_plan(
            PlanInput { current_kg: kg, target_kg: kg, pace: 0.0, tdee, bmr, sex: profile.sex },
            today,
        )),
        _ => None,
    };
    Vitals { age, current_kg, bmi, bmr, bmr_from_scale: scale_bmr.is_some(), tdee, water_goal_ml, water_goal_auto_ml, maintenance }
}

/// El plan que rige: el guardado o el de mantenimiento.
pub fn active_plan<'a>(data: &'a HealthData, vitals: &'a Vitals) -> Option<&'a Plan> {
    data.plan.as_ref().or(vitals.maintenance.as_ref())
}

pub fn meals_of(data: &HealthData, date: &str) -> Vec<Meal> {
    let mut meals = data.meals.iter().filter(|meal| meal.date == date).cloned().collect::<Vec<_>>();
    meals.sort_by_key(|meal| meal.created_at_ms);
    meals
}

pub fn total(meals: &[Meal]) -> Nutrition {
    meals.iter().fold(Nutrition::default(), |sum, meal| sum.add(meal.nutrition))
}

// ---------------------------------------------------------------------------
// Validación
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

pub fn field_error(field: &str, message: impl Into<String>) -> FieldError {
    FieldError { field: field.to_string(), message: message.into() }
}

fn check_date(field: &str, value: &str, today: Date, errors: &mut Vec<FieldError>) -> Option<Date> {
    match parse_date(value) {
        None => {
            errors.push(field_error(field, "Elegí una fecha válida."));
            None
        }
        Some(date) if date > today => {
            errors.push(field_error(field, "La fecha no puede ser futura."));
            None
        }
        Some(date) if date.year() < 1900 => {
            errors.push(field_error(field, "La fecha es demasiado antigua."));
            None
        }
        Some(date) => Some(date),
    }
}

fn check_weight(field: &str, kg: Option<f64>, errors: &mut Vec<FieldError>) {
    match kg {
        None => errors.push(field_error(field, "Escribí el peso en kg.")),
        Some(kg) if !(MIN_WEIGHT_KG..=MAX_WEIGHT_KG).contains(&kg) => {
            errors.push(field_error(field, format!("El peso tiene que estar entre {} y {} kg.", fmt(MIN_WEIGHT_KG, 0), fmt(MAX_WEIGHT_KG, 0))))
        }
        Some(_) => {}
    }
}

/// El formulario del perfil, con el peso actual.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub birth_date: String,
    pub sex: String,
    pub height_cm: Option<f64>,
    pub activity: Option<f64>,
    pub weight_kg: Option<f64>,
    /// Meta de agua propia en litros; vacía o 0, la calculada.
    #[serde(default)]
    pub water_goal_liters: Option<f64>,
}

/// La meta de agua propia permitida, en litros.
const WATER_GOAL_LITERS: std::ops::RangeInclusive<f64> = 0.5..=6.0;

pub fn validate_profile(input: &ProfileInput, today: Date) -> Result<(Profile, Option<f64>), Vec<FieldError>> {
    let mut errors = Vec::new();
    let birth = check_date("birthDate", &input.birth_date, today, &mut errors);
    if let Some(birth) = birth {
        let years = age(&birth.to_string(), today).unwrap_or_default();
        if years < 14 {
            errors.push(field_error("birthDate", "Salud es para personas de 14 años o más."));
        } else if years > 120 {
            errors.push(field_error("birthDate", "Revisá la fecha de nacimiento."));
        }
    }
    let sex = Sex::parse(&input.sex);
    if sex.is_none() {
        errors.push(field_error("sex", "Elegí el sexo biológico."));
    }
    match input.height_cm {
        Some(height) if (100.0..=250.0).contains(&height) => {}
        _ => errors.push(field_error("heightCm", "La altura tiene que estar entre 100 y 250 cm.")),
    }
    let activity = input.activity.unwrap_or(DEFAULT_ACTIVITY);
    if activity_label(activity).is_none() {
        errors.push(field_error("activity", "Elegí un nivel de actividad de la lista."));
    }
    if input.weight_kg.is_some() {
        check_weight("weightKg", input.weight_kg, &mut errors);
    }
    // De a 50 ml, como la calculada.
    let water_goal_ml = match input.water_goal_liters.filter(|liters| *liters != 0.0) {
        None => None,
        Some(liters) if WATER_GOAL_LITERS.contains(&liters) => Some(((liters * 1000.0 / 50.0).round() * 50.0) as i64),
        Some(_) => {
            errors.push(field_error("waterGoalLiters", "La meta de agua tiene que estar entre 0,5 y 6 litros. Dejala vacía para que se calcule."));
            None
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok((
        Profile {
            birth_date: birth.map(|date| date.to_string()).unwrap_or_default(),
            sex: sex.unwrap_or(Sex::Male),
            height_cm: round1(input.height_cm.unwrap_or_default()),
            activity,
            water_goal_ml,
        },
        input.weight_kg.map(round1),
    ))
}

pub fn validate_weight(date: &str, kg: Option<f64>, today: Date) -> Result<WeightEntry, Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", date, today, &mut errors);
    check_weight("kg", kg, &mut errors);
    match (date, kg) {
        (Some(date), Some(kg)) if errors.is_empty() => Ok(WeightEntry { date: date.to_string(), kg: round1(kg) }),
        _ => Err(errors),
    }
}

/// Una medición de la balanza tal como se carga: el peso es obligatorio,
/// los demás valores opcionales. Los ceros se toman como vacíos.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementInput {
    pub date: String,
    pub weight: Option<f64>,
    #[serde(default)]
    pub values: BTreeMap<String, f64>,
}

pub fn validate_measurement(input: &MeasurementInput, height_cm: Option<f64>, today: Date) -> Result<Measurement, Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", &input.date, today, &mut errors);
    check_weight("weight", input.weight, &mut errors);
    let mut values = BTreeMap::new();
    for (key, value) in &input.values {
        let Some(metric) = metric_def(key) else {
            errors.push(field_error(key, "Ese valor no es de la balanza."));
            continue;
        };
        if *value == 0.0 && metric.min > 0.0 {
            continue;
        }
        if !(metric.min..=metric.max).contains(value) {
            errors.push(field_error(key, format!("{}: tiene que estar entre {} y {}.", metric.label, fmt(metric.min, 1), fmt(metric.max, 1))));
            continue;
        }
        values.insert(key.clone(), if metric.decimals == 0 { value.round() } else { round1(*value) });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let measurement = Measurement {
        date: date.map(|date| date.to_string()).unwrap_or_default(),
        weight: round1(input.weight.unwrap_or_default()),
        values,
    };
    Ok(complete_measurement(&measurement, height_cm))
}

pub fn validate_objective(target_kg: Option<f64>, pace: Option<f64>) -> Result<Objective, Vec<FieldError>> {
    let mut errors = Vec::new();
    if let Some(target) = target_kg {
        if !(MIN_WEIGHT_KG..=MAX_WEIGHT_KG).contains(&target) {
            errors.push(field_error("targetKg", format!("El objetivo tiene que estar entre {} y {} kg.", fmt(MIN_WEIGHT_KG, 0), fmt(MAX_WEIGHT_KG, 0))));
        }
    }
    let pace = pace.unwrap_or(DEFAULT_PACE);
    if !PACES.iter().any(|option| (option - pace).abs() < 1e-6) {
        errors.push(field_error("pace", "El ritmo tiene que ser 0,25, 0,5, 0,75 o 1 kg por semana."));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Objective { target_kg: target_kg.map(round1), pace })
}

pub fn validate_water(date: &str, ml: i64, today: Date) -> Result<(String, i64), Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", date, today, &mut errors);
    if ml > MAX_WATER_ML {
        errors.push(field_error("ml", format!("El agua de un día no puede superar {} ml.", fmt(MAX_WATER_ML as f64, 0))));
    }
    match date {
        Some(date) if errors.is_empty() => Ok((date.to_string(), ml.max(0))),
        _ => Err(errors),
    }
}

/// Los pasos de un día, escritos como los escribe es-AR («8.432»).
pub fn validate_steps(date: &str, steps: &str, today: Date) -> Result<(String, i64), Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", date, today, &mut errors);
    let steps = parse_number(steps).map(|steps| steps.round() as i64);
    match steps {
        None => errors.push(field_error("steps", "Escribí la cantidad de pasos.")),
        Some(steps) if !(0..=MAX_STEPS).contains(&steps) => {
            errors.push(field_error("steps", format!("Los pasos de un día van de 0 a {}.", fmt(MAX_STEPS as f64, 0))))
        }
        Some(_) => {}
    }
    match (date, steps) {
        (Some(date), Some(steps)) if errors.is_empty() => Ok((date.to_string(), steps)),
        _ => Err(errors),
    }
}

/// Los minutos de una duración de sueño: «7,5» (horas), «7:30», «7 h 30»,
/// «7 horas y media», «450 min». `None` cuando no es una duración.
pub fn parse_sleep_minutes(text: &str) -> Option<i64> {
    let text = fold(text.trim());
    if let Some((hours, minutes)) = text.split_once(':') {
        let hours = hours.trim().parse::<i64>().ok()?;
        let minutes = minutes.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<i64>().ok()?;
        return (minutes < 60).then_some(hours * 60 + minutes);
    }
    // Cada número con la unidad que lo sigue (h, hora, m, min…).
    let mut parts: Vec<(f64, Option<char>)> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&character) = chars.peek() {
        if character.is_ascii_digit() {
            let mut number = String::new();
            while let Some(&digit) = chars.peek().filter(|c| c.is_ascii_digit() || **c == ',' || **c == '.') {
                number.push(digit);
                chars.next();
            }
            while chars.peek().is_some_and(|c| *c == ' ') {
                chars.next();
            }
            let unit = chars.peek().copied().filter(|c| *c == 'h' || *c == 'm');
            parts.push((number.replace(',', ".").trim_end_matches('.').parse::<f64>().ok()?, unit));
        } else {
            chars.next();
        }
    }
    let half = text.contains("media") || text.contains("medio");
    let minutes = match parts.as_slice() {
        [] => return None,
        [(minutes, Some('m'))] => *minutes,
        [(hours, _)] => hours * 60.0 + if half { 30.0 } else { 0.0 },
        [(hours, _), (minutes, _), ..] => hours * 60.0 + minutes,
    };
    minutes.is_finite().then(|| minutes.round() as i64)
}

/// La duración de sueño de un día.
pub fn validate_sleep(date: &str, duration: &str, today: Date) -> Result<(String, i64), Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", date, today, &mut errors);
    let minutes = parse_sleep_minutes(duration);
    match minutes {
        None => errors.push(field_error("duration", "Escribí cuánto dormiste: 7,5 o 7:30.")),
        Some(minutes) if !(1..=MAX_SLEEP_MINUTES).contains(&minutes) => {
            errors.push(field_error("duration", "El sueño de un día va de 1 minuto a 24 horas."))
        }
        Some(_) => {}
    }
    match (date, minutes) {
        (Some(date), Some(minutes)) if errors.is_empty() => Ok((date.to_string(), minutes)),
        _ => Err(errors),
    }
}

/// «7 h 30 min», «8 h», «45 min».
pub fn sleep_label(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// «8.432 pasos», «1 paso».
pub fn steps_label(steps: i64) -> String {
    format!("{} {}", fmt(steps as f64, 0), if steps == 1 { "paso" } else { "pasos" })
}

/// Una comida tal como se carga. Sin calorías, salen de los macros.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MealInput {
    pub date: String,
    pub category: String,
    pub name: String,
    pub kcal: Option<f64>,
    pub protein_g: Option<f64>,
    pub carbs_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub fiber_g: Option<f64>,
    /// The recipe it comes from; an edit without it keeps the stored one.
    #[serde(default)]
    pub recipe_id: Option<String>,
}

impl MealInput {
    pub fn has_nutrition(&self) -> bool {
        self.kcal.is_some_and(|kcal| kcal > 0.0)
            || [self.protein_g, self.carbs_g, self.fat_g].iter().any(|value| value.is_some_and(|value| value > 0.0))
    }
}

pub fn validate_meal(input: &MealInput, today: Date) -> Result<(String, MealCategory, String, Nutrition), Vec<FieldError>> {
    let mut errors = Vec::new();
    let date = check_date("date", &input.date, today, &mut errors);
    let category = MealCategory::parse(&input.category);
    if category.is_none() {
        errors.push(field_error("category", "Elegí desayuno, snack, almuerzo, merienda o cena."));
    }
    let name = input.name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        errors.push(field_error("name", "Escribí qué comiste."));
    } else if name.chars().count() > MAX_MEAL_NAME_CHARS {
        errors.push(field_error("name", format!("La descripción puede tener hasta {MAX_MEAL_NAME_CHARS} caracteres.")));
    }
    for (field, value, max) in [
        ("kcal", input.kcal, 10_000.0),
        ("proteinG", input.protein_g, 1_000.0),
        ("carbsG", input.carbs_g, 1_000.0),
        ("fatG", input.fat_g, 1_000.0),
        ("fiberG", input.fiber_g, 500.0),
    ] {
        if value.is_some_and(|value| !(0.0..=max).contains(&value)) {
            errors.push(field_error(field, format!("Tiene que estar entre 0 y {}.", fmt(max, 0))));
        }
    }
    let protein_g = round1(input.protein_g.unwrap_or_default());
    let carbs_g = round1(input.carbs_g.unwrap_or_default());
    let fat_g = round1(input.fat_g.unwrap_or_default());
    let kcal = input.kcal.filter(|kcal| *kcal > 0.0).map(f64::round).unwrap_or_else(|| kcal_from_macros(protein_g, carbs_g, fat_g));
    if errors.is_empty() && kcal <= 0.0 {
        errors.push(field_error("kcal", "Cargá las calorías o los macros, o pedile a la IA que los estime."));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok((
        date.map(|date| date.to_string()).unwrap_or_default(),
        category.unwrap_or(MealCategory::Almuerzo),
        name,
        Nutrition { kcal, protein_g, carbs_g, fat_g, fiber_g: round1(input.fiber_g.unwrap_or_default()) },
    ))
}

/// Mapea el momento de una comida al de Recetas (que no tiene merienda).
pub fn recipe_meal_time(category: MealCategory) -> crate::recipes::MealTime {
    use crate::recipes::MealTime;
    match category {
        MealCategory::Desayuno => MealTime::Breakfast,
        MealCategory::Almuerzo => MealTime::Lunch,
        MealCategory::Cena => MealTime::Dinner,
        MealCategory::Snack | MealCategory::Merienda => MealTime::Snack,
    }
}

/// Calorías y macros de una porción de una receta de Recetas.
pub fn recipe_serving(recipe: &crate::recipes::Recipe) -> Nutrition {
    Nutrition {
        kcal: recipe.kcal(),
        protein_g: recipe.value("prot"),
        carbs_g: recipe.value("carb"),
        fat_g: recipe.value("grasa"),
        fiber_g: recipe.value("fibra"),
    }
}

/// Una porción multiplicada por `factor` (1,5 porciones, 500 g sobre 350 g…).
pub fn scale(nutrition: Nutrition, factor: f64) -> Nutrition {
    Nutrition {
        kcal: (nutrition.kcal * factor).round(),
        protein_g: round1(nutrition.protein_g * factor),
        carbs_g: round1(nutrition.carbs_g * factor),
        fat_g: round1(nutrition.fat_g * factor),
        fiber_g: round1(nutrition.fiber_g * factor),
    }
}

/// Cuánto se comió de una receta.
#[derive(Debug, Clone, PartialEq)]
pub enum Portion {
    /// Una porción de la receta, tal cual.
    One,
    Servings(f64),
    Grams(f64),
    /// Otra cantidad o ingredientes distintos: la IA ajusta los valores.
    Note(String),
}

/// Las porciones de la receta que son `portion`; `None` cuando hace falta la
/// IA (una nota, o gramos sin el peso de la porción).
pub fn portion_factor(recipe: &crate::recipes::Recipe, portion: &Portion) -> Option<f64> {
    match portion {
        Portion::One => Some(1.0),
        Portion::Servings(servings) => Some(*servings),
        Portion::Grams(grams) => recipe.serving_grams.filter(|serving| *serving > 0.0).map(|serving| grams / serving),
        Portion::Note(_) => None,
    }
}

/// Las porciones de la receta que comió, o `None` si las tiene que calcular
/// la IA: con una nota, con gramos sin el peso de la porción, o con la foto
/// de una receta que ya existía (la foto muestra cuánto comió).
pub fn eaten_factor(recipe: &crate::recipes::Recipe, portion: &Portion, photo_of_existing: bool) -> Option<f64> {
    if photo_of_existing && *portion == Portion::One {
        return None;
    }
    portion_factor(recipe, portion)
}

/// Cómo salieron los valores de una comida, para la confirmación y el
/// resultado: el mismo cálculo da siempre lo mismo; lo que estima la IA (una
/// nota, gramos sin el peso de la porción, la foto de una receta que ya
/// existía) puede variar de una vez a otra.
pub fn meal_basis(recipe_name: &str, serving_kcal: f64, factor: Option<f64>, eaten: &str) -> String {
    let per_serving = format!("{} kcal por porción", fmt(serving_kcal, 0));
    match factor {
        Some(factor) => {
            let servings = if (factor - 1.0).abs() < 1e-6 { "1 porción".to_string() } else { format!("{} porciones", fmt(factor, 2)) };
            format!("Calculado: {servings} de «{}» ({per_serving}).", recipe_name.trim())
        }
        None => format!(
            "Estimado por la IA para «{}» sobre «{}» ({per_serving}); otra estimación puede dar distinto.",
            eaten.trim(),
            recipe_name.trim()
        ),
    }
}

/// El nombre de la comida en Salud: el de la receta y, si no fue una
/// porción, cuánto: «Guiso de lentejas · 500 g», «… · 1,5 porciones».
pub fn portion_name(recipe_name: &str, portion: &Portion) -> String {
    let suffix = match portion {
        Portion::One => String::new(),
        Portion::Servings(servings) if (servings - 1.0).abs() < 1e-6 => String::new(),
        Portion::Servings(servings) => format!(" · {} porciones", fmt(*servings, 2)),
        Portion::Grams(grams) => format!(" · {} g", fmt(*grams, 0)),
        Portion::Note(note) => format!(" ({})", note.trim()),
    };
    let name = format!("{}{suffix}", recipe_name.trim());
    if name.chars().count() > MAX_MEAL_NAME_CHARS {
        name.chars().take(MAX_MEAL_NAME_CHARS - 1).collect::<String>() + "…"
    } else {
        name
    }
}

/// Los errores de campo en un solo mensaje, para el modelo o un aviso.
pub fn field_errors_message(errors: &[FieldError]) -> String {
    errors.iter().map(|error| error.message.as_str()).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Cambios
// ---------------------------------------------------------------------------

/// Un cambio en los datos de Salud: lo mandan la pantalla y las herramientas
/// de la IA. El plan con IA y la estimación de una comida se resuelven antes
/// (necesitan al modelo) y llegan acá ya calculados.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HealthMutation {
    SaveProfile { input: ProfileInput },
    AddWeight { date: String, kg: Option<f64> },
    DeleteWeight { date: String },
    SaveMeasurement { input: MeasurementInput },
    DeleteMeasurement { date: String },
    SetObjective { target_kg: Option<f64>, pace: Option<f64> },
    /// Un plan calculado sin IA con el objetivo guardado.
    CalculatePlan,
    ClearPlan,
    AddWater { date: String, delta_ml: i64 },
    SetWater { date: String, ml: i64 },
    /// Los pasos de un día, como se escribieron («8.432»).
    SetSteps { date: String, steps: String },
    DeleteSteps { date: String },
    /// El sueño de un día, como se escribió («7,5», «7:30»).
    SetSleep { date: String, duration: String },
    DeleteSleep { date: String },
    SaveMeal { id: Option<String>, input: MealInput },
    DeleteMeal { id: String },
}
