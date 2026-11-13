//! Salud: guarda en la base de la biblioteca los datos de cada usuario
//! (perfil, objetivo y plan, pesos, mediciones de la balanza, agua y
//! comidas) y llama a la IA para el plan y para estimar una comida. Qué se
//! valida, qué se escribe y qué se muestra lo decide el núcleo
//! (`backend_core::health`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Timelike;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::backend::health::ai::{meal_prompt, parse_meal, parse_plan, parse_portion, plan_prompt, portion_prompt, PlanContext};
use crate::backend::health::change::{ai_plan_summary, plan_basis, plan_change, Change, HealthError, HealthErrorCode, NewIds, StoreOp};
use crate::backend::health::dashboard::{build_dashboard, DashboardQuery, HealthDashboard};
use crate::backend::health::tools::{records_view, summary_view, tool_action, MealRequest, ToolAction};
use crate::backend::health::{
    calorie_floor, eaten_factor, field_errors_message, fmt, meal_basis, parse_date, portion_name, recipe_meal_time, recipe_serving, scale, vitals, FieldError,
    HealthData, HealthMutation, Meal, MealCategory, MealInput, Measurement, Nutrition, Objective, Plan, Portion, Profile, WeightEntry,
};
use crate::backend::recipes::{find_duplicate, Recipe, RecipeInput};
use crate::recipes::{RecipesError, RecipesErrorCode};
use crate::backend::{BackendError, BackendErrorCode};
use crate::host::{AppHandle, Emitter, Manager};
use crate::library_registry::{LibraryBindingRegistry, LibraryBindingRoot};
use crate::recipes::MessageImage;

/// Rust avisa con el id de la biblioteca cuando cambian los datos de Salud.
pub(crate) const HEALTH_CHANGED_EVENT: &str = "notia://health-changed";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const AI_TIMEOUT: Duration = Duration::from_secs(120);
/// Un plan o una comida estimados para una confirmación, por llamada.
const PENDING_TTL: Duration = Duration::from_secs(30 * 60);

// ---------------------------------------------------------------------------
// Errores y contexto
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthCommandErrorCode {
    Validation,
    NotFound,
    Ai,
    Storage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCommandError {
    pub code: HealthCommandErrorCode,
    pub message: String,
    pub fields: Vec<FieldError>,
}

impl HealthCommandError {
    fn new(code: HealthCommandErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), fields: Vec::new() }
    }

    fn storage(message: impl Into<String>) -> Self {
        Self::new(HealthCommandErrorCode::Storage, message)
    }

    fn ai(message: impl Into<String>) -> Self {
        Self::new(HealthCommandErrorCode::Ai, message)
    }
}

impl From<HealthError> for HealthCommandError {
    fn from(error: HealthError) -> Self {
        let code = match error.code {
            HealthErrorCode::Validation => HealthCommandErrorCode::Validation,
            HealthErrorCode::NotFound => HealthCommandErrorCode::NotFound,
        };
        Self { code, message: error.message, fields: error.fields }
    }
}

impl From<RecipesError> for HealthCommandError {
    fn from(error: RecipesError) -> Self {
        let code = match error.code {
            RecipesErrorCode::Validation => HealthCommandErrorCode::Validation,
            RecipesErrorCode::NotFound => HealthCommandErrorCode::NotFound,
            RecipesErrorCode::Ai => HealthCommandErrorCode::Ai,
            RecipesErrorCode::Duplicate | RecipesErrorCode::Storage => HealthCommandErrorCode::Storage,
        };
        let message = if error.fields.is_empty() {
            error.message
        } else {
            error.fields.iter().map(|field| field.message.as_str()).collect::<Vec<_>>().join(" ")
        };
        Self::new(code, message)
    }
}

impl From<rusqlite::Error> for HealthCommandError {
    fn from(error: rusqlite::Error) -> Self {
        Self::storage(format!("No se pudo acceder a los datos de Salud: {error}"))
    }
}

impl From<BackendError> for HealthCommandError {
    fn from(error: BackendError) -> Self {
        let code = match error.code {
            BackendErrorCode::NotFound => HealthCommandErrorCode::NotFound,
            BackendErrorCode::InvalidInput => HealthCommandErrorCode::Validation,
            _ => HealthCommandErrorCode::Storage,
        };
        Self::new(code, error.message)
    }
}

pub type HealthResult<T> = Result<T, HealthCommandError>;

/// El error que recibe el modelo.
pub(crate) fn tool_error(error: HealthCommandError) -> BackendError {
    match error.code {
        HealthCommandErrorCode::Validation if !error.fields.is_empty() => BackendError::invalid_input(field_errors_message(&error.fields)),
        HealthCommandErrorCode::Validation => BackendError::invalid_input(error.message),
        HealthCommandErrorCode::NotFound => BackendError::new(BackendErrorCode::NotFound, error.message, false),
        HealthCommandErrorCode::Ai => BackendError::new(BackendErrorCode::ProviderUnavailable, error.message, true),
        HealthCommandErrorCode::Storage => BackendError::new(BackendErrorCode::Storage, error.message, true),
    }
}

/// Biblioteca y usuario de una operación: cada uno ve solo lo suyo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthContext {
    pub library_id: String,
    pub actor_library_user_id: String,
}

impl HealthContext {
    fn owner(&self) -> &str {
        self.actor_library_user_id.trim()
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as i64).unwrap_or_default()
}

/// La fecha y la hora locales del equipo.
fn local_now() -> (jiff::civil::Date, u32) {
    let now = chrono::Local::now();
    let today = parse_date(&now.format("%Y-%m-%d").to_string()).unwrap_or(jiff::civil::date(1970, 1, 1));
    (today, now.hour())
}

// ---------------------------------------------------------------------------
// Base de datos
// ---------------------------------------------------------------------------

struct Location {
    library_path: String,
    android_directory_uri: Option<String>,
}

fn location(app: &AppHandle, library_id: &str) -> HealthResult<Location> {
    let binding = app.state::<LibraryBindingRegistry>().lookup(library_id)?;
    match binding.root {
        Some(LibraryBindingRoot::Desktop { canonical_root }) => {
            Ok(Location { library_path: canonical_root.to_string_lossy().to_string(), android_directory_uri: None })
        }
        Some(LibraryBindingRoot::Android { tree_uri }) => {
            Ok(Location { library_path: String::new(), android_directory_uri: Some(tree_uri.as_str().to_string()) })
        }
        None => Err(HealthCommandError::storage("La biblioteca no está disponible.")),
    }
}

fn open(app: &AppHandle, context: &HealthContext, location: &Location) -> HealthResult<Connection> {
    if context.owner().is_empty() {
        return Err(HealthCommandError::new(HealthCommandErrorCode::Validation, "El usuario de la biblioteca es obligatorio para Salud."));
    }
    let connection = crate::database::open_user_data_connection(app, &location.library_path, location.android_directory_uri.as_deref())
        .map_err(HealthCommandError::storage)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM library_users WHERE id = ?1)", [context.owner()], |row| row.get(0))?;
    if !exists {
        return Err(HealthCommandError::new(HealthCommandErrorCode::Validation, "El usuario de la biblioteca no está autorizado para Salud."));
    }
    Ok(connection)
}

fn corrupt(what: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(0, what.to_string(), rusqlite::types::Type::Text)
}

fn json_column<T: serde::de::DeserializeOwned>(value: Option<String>, what: &str) -> rusqlite::Result<Option<T>> {
    value.map(|text| serde_json::from_str::<T>(&text).map_err(|_| corrupt(what))).transpose()
}

fn to_json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value).map_err(|_| corrupt("json"))
}

pub(crate) fn load(connection: &Connection, owner: &str) -> HealthResult<HealthData> {
    let settings = connection
        .query_row(
            "SELECT profile_json, objective_json, plan_json FROM health_settings WHERE owner_user_id = ?1",
            [owner],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .optional()?;
    let (profile, objective, plan) = match settings {
        Some((profile, objective, plan)) => (
            json_column::<Profile>(profile, "profile_json")?,
            json_column::<Objective>(objective, "objective_json")?,
            json_column::<Plan>(plan, "plan_json")?,
        ),
        None => (None, None, None),
    };
    let weights = connection
        .prepare("SELECT date, kg FROM health_weights WHERE owner_user_id = ?1 ORDER BY date")?
        .query_map([owner], |row| Ok(WeightEntry { date: row.get(0)?, kg: row.get(1)? }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let measurements = connection
        .prepare("SELECT date, weight, values_json FROM health_measurements WHERE owner_user_id = ?1 ORDER BY date")?
        .query_map([owner], |row| {
            let values: String = row.get(2)?;
            Ok(Measurement { date: row.get(0)?, weight: row.get(1)?, values: serde_json::from_str(&values).map_err(|_| corrupt("values_json"))? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let water = connection
        .prepare("SELECT date, ml FROM health_water WHERE owner_user_id = ?1")?
        .query_map([owner], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let steps = connection
        .prepare("SELECT date, steps FROM health_steps WHERE owner_user_id = ?1")?
        .query_map([owner], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let sleep = connection
        .prepare("SELECT date, minutes FROM health_sleep WHERE owner_user_id = ?1")?
        .query_map([owner], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let meals = connection
        .prepare(
            "SELECT id, date, category, name, kcal, protein_g, carbs_g, fat_g, fiber_g, created_at, recipe_id
             FROM health_meals WHERE owner_user_id = ?1 ORDER BY date, created_at",
        )?
        .query_map([owner], |row| {
            let category: String = row.get(2)?;
            Ok(Meal {
                id: row.get(0)?,
                date: row.get(1)?,
                category: MealCategory::parse(&category).ok_or_else(|| corrupt("category"))?,
                name: row.get(3)?,
                nutrition: Nutrition { kcal: row.get(4)?, protein_g: row.get(5)?, carbs_g: row.get(6)?, fat_g: row.get(7)?, fiber_g: row.get(8)? },
                created_at_ms: row.get(9)?,
                recipe_id: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let sleep_bpm = connection
        .prepare("SELECT date, bpm FROM health_sleep_heart WHERE owner_user_id = ?1")?
        .query_map([owner], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // Los días entrenados de Gimnasio, para sugerir el nivel de actividad.
    let exercise_dates = connection
        .prepare("SELECT DISTINCT date FROM gym_workouts WHERE owner_user_id = ?1")?
        .query_map([owner], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(HealthData {
        profile,
        weights,
        measurements,
        objective: objective.unwrap_or_default(),
        plan,
        water,
        steps,
        sleep,
        meals,
        sleep_bpm,
        exercise_dates,
    })
}

/// Crea la fila de ajustes del usuario si todavía no existe.
fn ensure_settings(connection: &Connection, owner: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO health_settings (owner_user_id, updated_at) VALUES (?1, ?2)",
        params![owner, now_ms()],
    )?;
    Ok(())
}

fn apply_ops(connection: &Connection, owner: &str, ops: &[StoreOp]) -> HealthResult<()> {
    let now = now_ms();
    for op in ops {
        match op {
            StoreOp::PutProfile(profile) => {
                ensure_settings(connection, owner)?;
                connection.execute(
                    "UPDATE health_settings SET profile_json = ?2, updated_at = ?3 WHERE owner_user_id = ?1",
                    params![owner, to_json(profile)?, now],
                )?;
            }
            StoreOp::PutObjective(objective) => {
                ensure_settings(connection, owner)?;
                connection.execute(
                    "UPDATE health_settings SET objective_json = ?2, updated_at = ?3 WHERE owner_user_id = ?1",
                    params![owner, to_json(objective)?, now],
                )?;
            }
            StoreOp::PutPlan(plan) => {
                ensure_settings(connection, owner)?;
                let plan = plan.as_ref().map(to_json).transpose()?;
                connection.execute(
                    "UPDATE health_settings SET plan_json = ?2, updated_at = ?3 WHERE owner_user_id = ?1",
                    params![owner, plan, now],
                )?;
            }
            StoreOp::PutWeight(entry) => {
                connection.execute(
                    "INSERT INTO health_weights (owner_user_id, date, kg, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET kg = excluded.kg, updated_at = excluded.updated_at",
                    params![owner, entry.date, entry.kg, now],
                )?;
            }
            StoreOp::DeleteWeight(date) => {
                connection.execute("DELETE FROM health_weights WHERE owner_user_id = ?1 AND date = ?2", params![owner, date])?;
            }
            StoreOp::PutMeasurement(measurement) => {
                connection.execute(
                    "INSERT INTO health_measurements (owner_user_id, date, weight, values_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET weight = excluded.weight, values_json = excluded.values_json,
                         updated_at = excluded.updated_at",
                    params![owner, measurement.date, measurement.weight, to_json(&measurement.values)?, now],
                )?;
            }
            StoreOp::DeleteMeasurement(date) => {
                connection.execute("DELETE FROM health_measurements WHERE owner_user_id = ?1 AND date = ?2", params![owner, date])?;
            }
            StoreOp::PutWater { date, ml } => {
                connection.execute(
                    "INSERT INTO health_water (owner_user_id, date, ml, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET ml = excluded.ml, updated_at = excluded.updated_at",
                    params![owner, date, ml, now],
                )?;
            }
            StoreOp::PutSteps { date, steps } => {
                connection.execute(
                    "INSERT INTO health_steps (owner_user_id, date, steps, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET steps = excluded.steps, updated_at = excluded.updated_at",
                    params![owner, date, steps, now],
                )?;
            }
            StoreOp::DeleteSteps(date) => {
                connection.execute("DELETE FROM health_steps WHERE owner_user_id = ?1 AND date = ?2", params![owner, date])?;
            }
            StoreOp::PutSleep { date, minutes } => {
                connection.execute(
                    "INSERT INTO health_sleep (owner_user_id, date, minutes, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET minutes = excluded.minutes, updated_at = excluded.updated_at",
                    params![owner, date, minutes, now],
                )?;
            }
            StoreOp::DeleteSleep(date) => {
                connection.execute("DELETE FROM health_sleep WHERE owner_user_id = ?1 AND date = ?2", params![owner, date])?;
            }
            StoreOp::PutSleepBpm { date, bpm } => {
                connection.execute(
                    "INSERT INTO health_sleep_heart (owner_user_id, date, bpm, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(owner_user_id, date) DO UPDATE SET bpm = excluded.bpm, updated_at = excluded.updated_at",
                    params![owner, date, bpm, now],
                )?;
            }
            StoreOp::PutMeal(meal) => {
                let n = &meal.nutrition;
                connection.execute(
                    "INSERT INTO health_meals (id, owner_user_id, date, category, name, kcal, protein_g, carbs_g, fat_g, fiber_g, created_at, updated_at, recipe_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT(id) DO UPDATE SET date = excluded.date, category = excluded.category, name = excluded.name,
                         kcal = excluded.kcal, protein_g = excluded.protein_g, carbs_g = excluded.carbs_g, fat_g = excluded.fat_g,
                         fiber_g = excluded.fiber_g, updated_at = excluded.updated_at, recipe_id = excluded.recipe_id
                     WHERE health_meals.owner_user_id = excluded.owner_user_id",
                    params![meal.id, owner, meal.date, meal.category.id(), meal.name, n.kcal, n.protein_g, n.carbs_g, n.fat_g, n.fiber_g, meal.created_at_ms, now, meal.recipe_id],
                )?;
            }
            StoreOp::DeleteMeal(id) => {
                connection.execute("DELETE FROM health_meals WHERE owner_user_id = ?1 AND id = ?2", params![owner, id])?;
            }
        }
    }
    Ok(())
}

fn read<T>(app: &AppHandle, context: &HealthContext, work: impl FnOnce(&HealthData) -> HealthResult<T>) -> HealthResult<T> {
    let location = location(app, &context.library_id)?;
    let connection = open(app, context, &location)?;
    work(&load(&connection, context.owner())?)
}

/// Aplica un cambio en una transacción y lo guarda (en Android, de vuelta
/// por SAF). `prepare` decide el cambio con los datos recién leídos.
fn write(app: &AppHandle, context: &HealthContext, prepare: impl FnOnce(&HealthData) -> HealthResult<Change>) -> HealthResult<Change> {
    let location = location(app, &context.library_id)?;
    let mut connection = open(app, context, &location)?;
    let transaction = connection.transaction()?;
    let data = load(&transaction, context.owner())?;
    let change = prepare(&data)?;
    apply_ops(&transaction, context.owner(), &change.ops)?;
    transaction.commit()?;
    drop(connection);
    crate::database::sync_user_data_connection(app, location.android_directory_uri.as_deref()).map_err(HealthCommandError::storage)?;
    let _ = app.emit(HEALTH_CHANGED_EVENT, context.library_id.as_str());
    Ok(change)
}

fn new_ids() -> NewIds {
    NewIds { meal_id: Uuid::new_v4().to_string(), now_ms: now_ms() }
}

// ---------------------------------------------------------------------------
// IA
// ---------------------------------------------------------------------------

/// El plan de la IA con el objetivo guardado. No guarda nada.
fn ai_plan(app: &AppHandle, library_id: &str, data: &HealthData, today: jiff::civil::Date) -> HealthResult<Plan> {
    let basis = plan_basis(data, today)?;
    let profile = data.profile.as_ref().ok_or_else(|| HealthCommandError::from(HealthError::validation("Configurá primero tu perfil.")))?;
    let floor = calorie_floor(basis.bmr, profile.sex);
    let context = PlanContext {
        profile,
        age: vitals(data, today).age,
        current_kg: basis.current_kg,
        target_kg: basis.target_kg,
        pace: data.objective.pace,
        bmr: basis.bmr,
        tdee: basis.tdee,
        floor,
        last_measurement: data.measurements.last(),
    };
    let (system, user) = plan_prompt(&context);
    let answer = crate::backend_runtime::complete_with_images(app, library_id, &system, &user, Vec::new(), AI_TIMEOUT)
        .map_err(|error| HealthCommandError::ai(format!("No se pudo generar el plan con IA ({}). Probá de nuevo o usá el cálculo sin IA.", error.message)))?;
    parse_plan(&answer, floor, today).ok_or_else(|| HealthCommandError::ai("La IA no devolvió un plan completo. Probá de nuevo o usá el cálculo sin IA."))
}

/// Calorías y macros estimados por la IA para una comida descripta (y su
/// foto si la hay), con un nombre si no lo tenía.
fn estimate(app: &AppHandle, library_id: &str, description: &str, photo: Option<String>) -> HealthResult<(Nutrition, String)> {
    let description = description.trim();
    if description.is_empty() && photo.is_none() {
        return Err(HealthError::validation("Escribí qué comiste para estimarlo.").into());
    }
    let (system, user) = meal_prompt(if description.is_empty() { "la comida de la foto" } else { description }, photo.is_some());
    let answer = crate::backend_runtime::complete_with_images(app, library_id, &system, &user, photo.into_iter().collect(), AI_TIMEOUT)
        .map_err(|error| HealthCommandError::ai(format!("No se pudo estimar con IA ({}). Probá con una descripción más concreta o cargá los valores a mano.", error.message)))?;
    parse_meal(&answer).ok_or_else(|| HealthCommandError::ai("La IA no devolvió los valores de la comida. Probá con una descripción más concreta o cargá los valores a mano."))
}

// ---------------------------------------------------------------------------
// Comandos
// ---------------------------------------------------------------------------

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> HealthResult<T> + Send + 'static) -> HealthResult<T> {
    crate::host::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| HealthCommandError::storage("No se pudo acceder a los datos de Salud."))?
}

fn dashboard(app: &AppHandle, context: &HealthContext, query: &DashboardQuery) -> HealthResult<HealthDashboard> {
    let (today, hour) = local_now();
    read(app, context, |data| Ok(build_dashboard(data, query, today, hour)))
}

pub async fn health_dashboard(app: AppHandle, context: HealthContext, query: Option<DashboardQuery>) -> HealthResult<HealthDashboard> {
    blocking(move || dashboard(&app, &context, &query.unwrap_or_default())).await
}

/// Aplica un cambio de la pantalla y devuelve el tablero actualizado.
pub async fn health_apply(app: AppHandle, context: HealthContext, mutation: HealthMutation, query: Option<DashboardQuery>) -> HealthResult<HealthDashboard> {
    blocking(move || {
        let (today, _) = local_now();
        write(&app, &context, |data| Ok(plan_change(data, &mutation, today, &new_ids())?))?;
        dashboard(&app, &context, &query.unwrap_or_default())
    })
    .await
}

/// Pide el plan a la IA, lo guarda y devuelve el tablero actualizado.
pub async fn health_generate_plan(app: AppHandle, context: HealthContext, query: Option<DashboardQuery>) -> HealthResult<HealthDashboard> {
    blocking(move || {
        let (today, _) = local_now();
        let plan = read(&app, &context, |data| ai_plan(&app, &context.library_id, data, today))?;
        write(&app, &context, |_| Ok(Change { ops: vec![StoreOp::PutPlan(Some(plan))], summary: String::new(), meal_id: None }))?;
        dashboard(&app, &context, &query.unwrap_or_default())
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MealEstimate {
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
}

/// «Estimar con IA» del formulario: los valores para completar los campos.
pub async fn health_estimate_meal(app: AppHandle, context: HealthContext, description: String) -> HealthResult<MealEstimate> {
    blocking(move || {
        location(&app, &context.library_id)?;
        let (nutrition, _) = estimate(&app, &context.library_id, &description, None)?;
        Ok(MealEstimate {
            kcal: nutrition.kcal,
            protein_g: nutrition.protein_g,
            carbs_g: nutrition.carbs_g,
            fat_g: nutrition.fat_g,
            fiber_g: nutrition.fiber_g,
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// Herramientas del agente
// ---------------------------------------------------------------------------

pub(crate) fn is_health_tool(name: &str) -> bool {
    crate::backend::health::tools::is_health_tool(name)
}

pub(crate) fn is_health_write_tool(name: &str) -> bool {
    crate::backend::health::tools::is_health_write_tool(name)
}

/// Lo que se confirma: el cambio con lo que ya calculó la IA.
#[derive(Debug, Clone)]
enum Ready {
    Mutation(HealthMutation),
    Plan(Plan),
    /// Lo que comió: la receta (nueva, que se guarda al confirmar, o la del
    /// recetario), la comida de Salud calculada con ella y cómo salieron sus
    /// valores (`meal_basis`).
    Meal { new_recipe: Option<Recipe>, recipe_name: String, meal: HealthMutation, basis: String },
}

/// Planes y comidas que la IA estimó para una confirmación, por llamada,
/// así se guarda exactamente lo que vio la persona.
fn pending() -> &'static Mutex<HashMap<String, (Instant, Ready)>> {
    static PENDING: OnceLock<Mutex<HashMap<String, (Instant, Ready)>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

fn keep_pending(call_id: &str, ready: &Ready) {
    if let Ok(mut pending) = pending().lock() {
        pending.retain(|_, (at, _)| at.elapsed() < PENDING_TTL);
        pending.insert(call_id.to_string(), (Instant::now(), ready.clone()));
    }
}

fn take_pending(call_id: &str) -> Option<Ready> {
    pending().lock().ok()?.remove(call_id).filter(|(at, _)| at.elapsed() < PENDING_TTL).map(|(_, ready)| ready)
}

/// Resuelve una llamada de escritura: con la IA si hace falta.
fn resolve(app: &AppHandle, context: &HealthContext, data: &HealthData, name: &str, arguments: &Value, images: &[MessageImage]) -> HealthResult<Ready> {
    let (today, hour) = local_now();
    match tool_action(name, arguments, data, today, hour)? {
        ToolAction::Change(mutation) => Ok(Ready::Mutation(mutation)),
        ToolAction::AiPlan => Ok(Ready::Plan(ai_plan(app, &context.library_id, data, today)?)),
        ToolAction::Meal(request) => resolve_meal(app, context, request, images, hour),
    }
}

/// Lo que comió la persona, en Recetas y en Salud:
/// 1. la receta que nombró, o la del recetario con el mismo nombre (sin
///    acentos, plurales ni conectores); sin nombre, la IA reconoce el plato
///    de la foto;
/// 2. si no existe, una receta nueva armada a partir del plato (con la foto
///    si la hay), revisada por la IA; si la IA ve que repite una existente,
///    se usa esa;
/// 3. la comida de Salud con los valores de la cantidad que comió: una
///    porción, varias, un peso sobre el de la porción, o lo que la IA ajusta
///    para una nota («con doble papas») o para la foto de una receta que ya
///    existía. Los valores que dijo la persona mandan.
/// No guarda nada.
fn resolve_meal(app: &AppHandle, context: &HealthContext, request: MealRequest, images: &[MessageImage], hour: u32) -> HealthResult<Ready> {
    let library_id = context.library_id.as_str();
    let photo = request.photo.map(|number| crate::recipes::photo_from_message(images, number)).transpose()?;
    let recipes = crate::recipes::load_recipes(app, library_id)?;
    let mut existing = match &request.recipe {
        Some(reference) => Some(
            crate::backend::recipes::tools::resolve_recipe(&recipes, &serde_json::json!({ "recipe": reference }))
                .map_err(HealthCommandError::from)?
                .clone(),
        ),
        None => None,
    };
    let mut name = request.input.name.trim().to_string();
    if existing.is_none() && name.is_empty() {
        let Some(photo) = &photo else {
            return Err(HealthError::validation("Falta qué comió.").into());
        };
        name = estimate(app, library_id, "", Some(photo.base64.clone()))?.1;
        if name.trim().is_empty() {
            return Err(HealthError::validation("No se pudo reconocer la comida de la foto: preguntale qué es.").into());
        }
    }
    if existing.is_none() {
        existing = find_duplicate(&name, &recipes, None).cloned();
    }
    let category = MealCategory::parse(&request.input.category).unwrap_or_else(|| MealCategory::for_hour(hour));
    let mut new_recipe = None;
    let recipe = match existing {
        Some(recipe) => recipe,
        None => {
            let input = RecipeInput {
                name: name.clone(),
                meal: Some(recipe_meal_time(category)),
                description: request.description.clone(),
                ingredients: request.ingredients.clone(),
                ..RecipeInput::default()
            };
            match crate::recipes::prepare_new(app, library_id, &input, photo.clone(), &recipes, true) {
                Ok(recipe) => {
                    new_recipe = Some(recipe.clone());
                    recipe
                }
                // The AI saw that it repeats a recipe: that one is used.
                Err(error) if error.code == RecipesErrorCode::Duplicate => recipes
                    .iter()
                    .find(|recipe| Some(&recipe.id) == error.duplicate_id.as_ref())
                    .cloned()
                    .ok_or_else(|| HealthCommandError::from(error))?,
                Err(error) => return Err(error.into()),
            }
        }
    };

    let mut input = MealInput {
        name: portion_name(&recipe.name, &request.portion),
        category: category.id().to_string(),
        recipe_id: Some(recipe.id.clone()),
        ..request.input.clone()
    };
    let mut basis = "Con los valores que dijiste.".to_string();
    if !request.input.has_nutrition() {
        let serving = recipe_serving(&recipe);
        let photo_of_existing = new_recipe.is_none() && photo.is_some();
        let factor = eaten_factor(&recipe, &request.portion, photo_of_existing);
        let eaten = match &request.portion {
            Portion::Note(note) => note.clone(),
            Portion::Grams(grams) => format!("{} g", fmt(*grams, 0)),
            Portion::Servings(servings) => format!("{} porciones", fmt(*servings, 2)),
            Portion::One if request.description.trim().is_empty() => "la foto del plato".to_string(),
            Portion::One => request.description.clone(),
        };
        basis = meal_basis(&recipe.name, serving.kcal, factor, &eaten);
        let nutrition = match factor {
            Some(factor) => scale(serving, factor),
            None => {
                let (system, user) = portion_prompt(&recipe, &eaten, photo.is_some());
                let answer = crate::backend_runtime::complete_with_images(app, library_id, &system, &user, photo.iter().map(|photo| photo.base64.clone()).collect(), AI_TIMEOUT)
                    .map_err(|error| HealthCommandError::ai(format!("No se pudo calcular lo que comió con IA ({}).", error.message)))?;
                parse_portion(&answer, serving).ok_or_else(|| HealthCommandError::ai("La IA no devolvió los valores de lo que comió."))?
            }
        };
        input = MealInput {
            kcal: Some(nutrition.kcal),
            protein_g: Some(nutrition.protein_g),
            carbs_g: Some(nutrition.carbs_g),
            fat_g: Some(nutrition.fat_g),
            fiber_g: Some(nutrition.fiber_g),
            ..input
        };
    }
    Ok(Ready::Meal { new_recipe, recipe_name: recipe.name.clone(), meal: HealthMutation::SaveMeal { id: None, input }, basis })
}

/// Lo que se hace con la receta, para la confirmación.
fn recipe_summary(new_recipe: Option<&Recipe>, recipe_name: &str) -> String {
    match new_recipe {
        Some(recipe) => {
            let servings = match (recipe.servings, recipe.serving_grams) {
                (Some(servings), Some(grams)) => format!("{servings} porción(es) de {} g", fmt(grams, 0)),
                (Some(servings), None) => format!("{servings} porción(es)"),
                (None, Some(grams)) => format!("porción de {} g", fmt(grams, 0)),
                (None, None) => "sin porciones".to_string(),
            };
            format!(
                "Guardar en Recetas «{}» ({servings}, {} kcal por porción{}). Ingredientes: {}.",
                recipe.name,
                fmt(recipe.kcal(), 0),
                if recipe.photo.is_some() { ", con la foto" } else { "" },
                if recipe.ingredients.is_empty() { "—".to_string() } else { recipe.ingredients.join(", ") }
            )
        }
        None => format!("Usa la receta «{recipe_name}» del recetario."),
    }
}

fn summary_of(data: &HealthData, ready: &Ready) -> HealthResult<String> {
    let (today, _) = local_now();
    Ok(match ready {
        Ready::Mutation(mutation) => {
            let mut summary = plan_change(data, mutation, today, &new_ids())?.summary;
            if matches!(mutation, HealthMutation::SaveMeal { id: None, .. }) {
                summary.push_str(" Valores estimados si no los dijiste.");
            }
            summary
        }
        Ready::Plan(plan) => ai_plan_summary(plan),
        Ready::Meal { new_recipe, recipe_name, meal, basis } => {
            format!("{}\n{} {basis}", recipe_summary(new_recipe.as_ref(), recipe_name), plan_change(data, meal, today, &new_ids())?.summary)
        }
    })
}

/// La confirmación de una llamada: se valida (y se estima con IA) sin
/// guardar; un rechazo vuelve al modelo antes de preguntarle a la persona.
pub(crate) fn preview_tool(app: &AppHandle, context: &HealthContext, call_id: &str, name: &str, arguments: &Value, images: &[MessageImage]) -> HealthResult<String> {
    read(app, context, |data| {
        let ready = resolve(app, context, data, name, arguments, images)?;
        let summary = summary_of(data, &ready)?;
        keep_pending(call_id, &ready);
        Ok(summary)
    })
}

pub(crate) fn execute_tool(app: &AppHandle, context: &HealthContext, call_id: &str, name: &str, arguments: &Value, images: &[MessageImage]) -> HealthResult<Value> {
    let (today, _) = local_now();
    match name {
        "get_health_summary" => return read(app, context, |data| Ok(summary_view(data, today))),
        "list_health_records" => return read(app, context, |data| Ok(records_view(data, arguments, today)?)),
        _ => {}
    }
    let mut ready = match take_pending(call_id) {
        Some(ready) => ready,
        None => read(app, context, |data| resolve(app, context, data, name, arguments, images))?,
    };
    // The new recipe is saved first; one saved meanwhile with the same dish
    // is used instead.
    let mut recipe_id = None;
    if let Ready::Meal { new_recipe: Some(recipe), meal: HealthMutation::SaveMeal { input, .. }, .. } = &mut ready {
        match crate::recipes::save_new(app, &context.library_id, recipe) {
            Ok(()) => {}
            Err(error) if error.code == RecipesErrorCode::Duplicate && error.duplicate_id.is_some() => input.recipe_id = error.duplicate_id,
            Err(error) => return Err(error.into()),
        }
        recipe_id = input.recipe_id.clone();
    }
    let change = write(app, context, |data| match &ready {
        Ready::Mutation(mutation) | Ready::Meal { meal: mutation, .. } => Ok(plan_change(data, mutation, today, &new_ids())?),
        Ready::Plan(plan) => Ok(Change { ops: vec![StoreOp::PutPlan(Some(plan.clone()))], summary: ai_plan_summary(plan), meal_id: None }),
    })?;
    let recipe_id = recipe_id.or_else(|| match &ready {
        Ready::Meal { meal: HealthMutation::SaveMeal { input, .. }, .. } => input.recipe_id.clone(),
        _ => None,
    });
    let basis = match &ready {
        Ready::Meal { basis, .. } => Some(basis.clone()),
        _ => None,
    };
    // The day as it stands after the change (steps, water, what is left to
    // eat), so the answer does not need another get_health_summary.
    let health_today = read(app, context, |data| Ok(summary_view(data, today)))?;
    Ok(serde_json::json!({ "ok": true, "summary": change.summary, "mealId": change.meal_id, "recipeId": recipe_id, "basis": basis, "healthToday": health_today }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().expect("database");
        crate::database::migrate(&connection).expect("migrate");
        connection
            .execute(
                "INSERT INTO library_users (id, name, normalized_name, role_id, created_at, updated_at)
                 SELECT 'user-ana', 'Ana', 'ana', role_id, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP FROM library_users WHERE id = 'user-owner'",
                [],
            )
            .expect("user");
        connection
    }

    /// The watch's sleeping pulse is stored and read back by date, and the
    /// days trained in Gimnasio reach Salud (for the activity level).
    #[test]
    fn the_watch_pulse_and_the_training_days_round_trip() {
        let connection = database();
        let owner = crate::backend::OWNER_LIBRARY_USER_ID;
        let day = |date: &str, bpm: f64| crate::backend::health::WatchDay { date: date.into(), sleep_bpm: Some(bpm), ..Default::default() };
        apply(&connection, owner, HealthMutation::ImportWatch { days: vec![day("2026-09-27", 51.0), day("2026-09-28", 54.5)] });
        apply(&connection, owner, HealthMutation::ImportWatch { days: vec![day("2026-09-28", 53.0)] });
        connection
            .execute(
                "INSERT INTO gym_workouts (id, owner_user_id, date, ended_at, workout_json) VALUES ('w1', ?1, '2026-09-26', 1, '{}'), ('w2', ?1, '2026-09-26', 2, '{}'), ('w3', 'user-ana', '2026-09-25', 3, '{}')",
                [owner],
            )
            .expect("workouts");
        let data = load(&connection, owner).expect("load");
        assert_eq!(data.sleep_bpm.iter().map(|(date, bpm)| (date.as_str(), *bpm)).collect::<Vec<_>>(), [("2026-09-27", 51.0), ("2026-09-28", 53.0)]);
        assert_eq!(data.exercise_dates.iter().map(String::as_str).collect::<Vec<_>>(), ["2026-09-26"]);
    }

    fn apply(connection: &Connection, owner: &str, mutation: HealthMutation) -> Change {
        let today = jiff::civil::date(2026, 9, 28);
        let data = load(connection, owner).expect("load");
        let change = plan_change(&data, &mutation, today, &NewIds { meal_id: format!("meal-{}", data.meals.len()), now_ms: 10 }).expect("change");
        apply_ops(connection, owner, &change.ops).expect("apply");
        change
    }

    #[test]
    fn every_record_round_trips_through_the_database() {
        let connection = database();
        let owner = crate::backend::OWNER_LIBRARY_USER_ID;
        apply(
            &connection,
            owner,
            HealthMutation::SaveProfile {
                input: crate::backend::health::ProfileInput {
                    birth_date: "1992-03-15".into(),
                    sex: "M".into(),
                    height_cm: Some(178.0),
                    activity: Some(1.375),
                    weight_kg: Some(82.4),
                    water_goal_liters: None,
                },
            },
        );
        apply(&connection, owner, HealthMutation::SetObjective { target_kg: Some(76.0), pace: Some(0.5) });
        apply(&connection, owner, HealthMutation::CalculatePlan);
        apply(
            &connection,
            owner,
            HealthMutation::SaveMeasurement {
                input: crate::backend::health::MeasurementInput {
                    date: "2026-09-27".into(),
                    weight: Some(82.6),
                    values: [("grasaPct".to_string(), 22.0)].into(),
                },
            },
        );
        apply(&connection, owner, HealthMutation::AddWater { date: "2026-09-28".into(), delta_ml: 250 });
        apply(&connection, owner, HealthMutation::AddWater { date: "2026-09-28".into(), delta_ml: 500 });
        apply(&connection, owner, HealthMutation::SetSteps { date: "2026-09-27".into(), steps: "6.200".into() });
        apply(&connection, owner, HealthMutation::SetSteps { date: "2026-09-28".into(), steps: "9000".into() });
        apply(&connection, owner, HealthMutation::SetSteps { date: "2026-09-28".into(), steps: "9500".into() });
        apply(&connection, owner, HealthMutation::DeleteSteps { date: "2026-09-27".into() });
        apply(&connection, owner, HealthMutation::SetSleep { date: "2026-09-28".into(), duration: "7:15".into() });
        let meal = MealInput {
            date: "2026-09-28".into(),
            category: "almuerzo".into(),
            name: "Guiso".into(),
            kcal: Some(450.0),
            recipe_id: Some("recipe-guiso".into()),
            ..MealInput::default()
        };
        let added = apply(&connection, owner, HealthMutation::SaveMeal { id: None, input: meal.clone() });
        apply(&connection, owner, HealthMutation::SaveMeal { id: added.meal_id.clone(), input: MealInput { kcal: Some(480.0), ..meal } });

        let data = load(&connection, owner).expect("load");
        assert_eq!(data.profile.as_ref().map(|profile| profile.height_cm), Some(178.0));
        assert_eq!(data.weights.iter().map(|entry| (entry.date.as_str(), entry.kg)).collect::<Vec<_>>(), [("2026-09-27", 82.6), ("2026-09-28", 82.4)]);
        assert_eq!(data.objective.target_kg, Some(76.0));
        assert_eq!(data.plan.as_ref().map(|plan| plan.kcal), Some(1886.0));
        assert_eq!(data.measurements[0].value("grasaKg"), Some(18.2));
        assert_eq!(data.water.get("2026-09-28"), Some(&750));
        assert_eq!(data.steps.iter().map(|(date, steps)| (date.as_str(), *steps)).collect::<Vec<_>>(), [("2026-09-28", 9_500)]);
        assert_eq!(data.sleep.get("2026-09-28"), Some(&435));
        assert_eq!(data.meals.len(), 1);
        assert_eq!(data.meals[0].nutrition.kcal, 480.0);
        assert_eq!(data.meals[0].recipe_id.as_deref(), Some("recipe-guiso"));

        // Otro usuario no ve ni toca estos datos.
        assert_eq!(load(&connection, "user-ana").expect("load").meals.len(), 0);
        apply_ops(&connection, "user-ana", &[StoreOp::DeleteMeal(data.meals[0].id.clone())]).expect("delete");
        assert_eq!(load(&connection, owner).expect("load").meals.len(), 1);

        apply(&connection, owner, HealthMutation::ClearPlan);
        apply(&connection, owner, HealthMutation::DeleteMeasurement { date: "2026-09-27".into() });
        let data = load(&connection, owner).expect("load");
        assert!(data.plan.is_none() && data.measurements.is_empty() && data.weights.len() == 2);
    }

    #[test]
    fn a_pending_estimate_is_used_once() {
        keep_pending("call-1", &Ready::Mutation(HealthMutation::ClearPlan));
        assert!(matches!(take_pending("call-1"), Some(Ready::Mutation(HealthMutation::ClearPlan))));
        assert!(take_pending("call-1").is_none());
    }
}
