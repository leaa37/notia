//! Qué hace cada cambio de Salud: lo valida sobre los datos guardados y
//! devuelve las escrituras que tiene que hacer la app y un resumen para la
//! persona. Las herramientas de la IA muestran ese mismo resumen al pedir
//! confirmación.

use jiff::civil::Date;
use serde::Serialize;

use super::{
    calculate_plan, field_errors_message, fmt, parse_date, short_date, signed, validate_meal, validate_measurement, validate_objective,
    validate_profile, validate_sleep, validate_steps, validate_water, validate_weight, vitals, sleep_label, steps_label, FieldError, HealthData, HealthMutation, Meal, Measurement, Objective, Plan,
    PlanInput, Profile, WatchDay, WeightEntry,
};

#[derive(Debug, Clone, PartialEq)]
pub enum StoreOp {
    PutProfile(Profile),
    PutWeight(WeightEntry),
    DeleteWeight(String),
    PutMeasurement(Measurement),
    DeleteMeasurement(String),
    PutObjective(Objective),
    PutPlan(Option<Plan>),
    PutWater { date: String, ml: i64 },
    PutSteps { date: String, steps: i64 },
    DeleteSteps(String),
    PutSleep { date: String, minutes: i64 },
    DeleteSleep(String),
    /// El pulso promedio mientras dormía, del reloj.
    PutSleepBpm { date: String, bpm: f64 },
    PutMeal(Meal),
    DeleteMeal(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub ops: Vec<StoreOp>,
    pub summary: String,
    /// Id de la comida creada o cambiada.
    pub meal_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthErrorCode {
    Validation,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthError {
    pub code: HealthErrorCode,
    pub message: String,
    pub fields: Vec<FieldError>,
}

impl HealthError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self { code: HealthErrorCode::Validation, message: message.into(), fields: Vec::new() }
    }

    pub fn fields(fields: Vec<FieldError>) -> Self {
        Self { code: HealthErrorCode::Validation, message: field_errors_message(&fields), fields }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self { code: HealthErrorCode::NotFound, message: message.into(), fields: Vec::new() }
    }
}

/// Lo que piden los cambios para crear una comida.
#[derive(Debug, Clone)]
pub struct NewIds {
    pub meal_id: String,
    pub now_ms: i64,
}

fn date_label(date: &str) -> String {
    parse_date(date).map(short_date).unwrap_or_else(|| date.to_string())
}

fn kg(value: f64) -> String {
    format!("{} kg", fmt(value, 1))
}

pub fn plan_change(data: &HealthData, mutation: &HealthMutation, today: Date, ids: &NewIds) -> Result<Change, HealthError> {
    let change = |ops: Vec<StoreOp>, summary: String| Change { ops, summary, meal_id: None };
    match mutation {
        HealthMutation::SaveProfile { input } => {
            let (mut profile, weight) = validate_profile(input, today).map_err(HealthError::fields)?;
            // El formulario no trae esta elección: queda como estaba.
            profile.use_measured_expenditure = data.profile.as_ref().is_some_and(|stored| stored.use_measured_expenditure);
            let mut ops = vec![StoreOp::PutProfile(profile.clone())];
            let current = vitals(data, today).current_kg;
            let mut summary = format!(
                "{} el perfil: nacimiento {}, {}, {} cm, actividad «{}».",
                if data.profile.is_some() { "Actualizar" } else { "Guardar" },
                date_label(&profile.birth_date),
                profile.sex.label().to_lowercase(),
                fmt(profile.height_cm, 0),
                super::activity_label(profile.activity).unwrap_or_default()
            );
            if let Some(weight) = weight.filter(|weight| current.map_or(true, |current| (current - weight).abs() > 0.05)) {
                ops.push(StoreOp::PutWeight(WeightEntry { date: today.to_string(), kg: weight }));
                summary.push_str(&format!(" Peso de hoy: {}.", kg(weight)));
            }
            Ok(change(ops, summary))
        }
        HealthMutation::AddWeight { date, kg: value } => {
            let entry = validate_weight(date, *value, today).map_err(HealthError::fields)?;
            let before = data.weights.iter().find(|item| item.date == entry.date);
            let summary = match before {
                Some(before) => format!("Cambiar el peso del {}: {} → {}.", date_label(&entry.date), kg(before.kg), kg(entry.kg)),
                None => format!("Registrar {} el {}.", kg(entry.kg), date_label(&entry.date)),
            };
            Ok(change(vec![StoreOp::PutWeight(entry)], summary))
        }
        HealthMutation::DeleteWeight { date } => {
            let entry = data
                .weights
                .iter()
                .find(|item| item.date == date.trim())
                .ok_or_else(|| HealthError::not_found(format!("No hay un peso registrado el {}.", date_label(date))))?;
            Ok(change(vec![StoreOp::DeleteWeight(entry.date.clone())], format!("Eliminar el peso del {} ({}).", date_label(&entry.date), kg(entry.kg))))
        }
        HealthMutation::SaveMeasurement { input } => {
            let height = data.profile.as_ref().map(|profile| profile.height_cm);
            let measurement = validate_measurement(input, height, today).map_err(HealthError::fields)?;
            let replaces = data.measurements.iter().any(|item| item.date == measurement.date);
            let details = measurement
                .values
                .iter()
                .filter_map(|(key, value)| super::metric_def(key).map(|metric| (metric, value)))
                .take(6)
                .map(|(metric, value)| format!("{} {}{}", metric.label.to_lowercase(), fmt(*value, metric.decimals), if metric.unit.is_empty() { String::new() } else { format!(" {}", metric.unit) }))
                .collect::<Vec<_>>();
            let summary = format!(
                "{} la medición de la balanza del {}: {}{}.",
                if replaces { "Reemplazar" } else { "Registrar" },
                date_label(&measurement.date),
                kg(measurement.weight),
                if details.is_empty() { String::new() } else { format!(", {}", details.join(", ")) }
            );
            let weight = WeightEntry { date: measurement.date.clone(), kg: measurement.weight };
            Ok(change(vec![StoreOp::PutMeasurement(measurement), StoreOp::PutWeight(weight)], summary))
        }
        HealthMutation::DeleteMeasurement { date } => {
            let measurement = data
                .measurements
                .iter()
                .find(|item| item.date == date.trim())
                .ok_or_else(|| HealthError::not_found(format!("No hay una medición de la balanza del {}.", date_label(date))))?;
            Ok(change(
                vec![StoreOp::DeleteMeasurement(measurement.date.clone())],
                format!("Eliminar la medición de la balanza del {} (el peso de ese día queda registrado).", date_label(&measurement.date)),
            ))
        }
        HealthMutation::SetObjective { target_kg, pace } => {
            let objective = validate_objective(*target_kg, *pace).map_err(HealthError::fields)?;
            let summary = match objective.target_kg {
                Some(target) => format!("Fijar el peso objetivo en {} a {} kg por semana.", kg(target), fmt(objective.pace, 2)),
                None => format!("Quitar el peso objetivo (ritmo {} kg por semana).", fmt(objective.pace, 2)),
            };
            Ok(change(vec![StoreOp::PutObjective(objective)], summary))
        }
        HealthMutation::CalculatePlan => {
            let plan = calculated_plan(data, today)?;
            let summary = format!("Guardar un plan calculado de {} kcal por día. {}", fmt(plan.kcal, 0), plan.summary);
            Ok(change(vec![StoreOp::PutPlan(Some(plan))], summary))
        }
        HealthMutation::ClearPlan => {
            if data.plan.is_none() {
                return Err(HealthError::not_found("No hay un plan guardado: ya se usan los objetivos de mantenimiento."));
            }
            Ok(change(vec![StoreOp::PutPlan(None)], "Quitar el plan y volver a los objetivos de mantenimiento.".to_string()))
        }
        HealthMutation::AddWater { date, delta_ml } => {
            let current = data.water.get(date.trim()).copied().unwrap_or_default();
            let (date, ml) = validate_water(date, current + delta_ml, today).map_err(HealthError::fields)?;
            let summary = format!("{} {} ml de agua el {} (total {} ml).", if *delta_ml >= 0 { "Sumar" } else { "Restar" }, delta_ml.abs(), date_label(&date), ml);
            Ok(change(vec![StoreOp::PutWater { date, ml }], summary))
        }
        HealthMutation::SetWater { date, ml } => {
            let (date, ml) = validate_water(date, *ml, today).map_err(HealthError::fields)?;
            let summary = format!("Dejar el agua del {} en {} ml.", date_label(&date), ml);
            Ok(change(vec![StoreOp::PutWater { date, ml }], summary))
        }
        HealthMutation::SetSteps { date, steps } => {
            let (date, steps) = validate_steps(date, steps, today).map_err(HealthError::fields)?;
            let summary = match data.steps.get(&date) {
                Some(before) => format!("Cambiar los pasos del {}: {} → {}.", date_label(&date), steps_label(*before), steps_label(steps)),
                None => format!("Registrar {} el {}.", steps_label(steps), date_label(&date)),
            };
            Ok(change(vec![StoreOp::PutSteps { date, steps }], summary))
        }
        HealthMutation::DeleteSteps { date } => {
            let steps = data.steps.get(date.trim()).ok_or_else(|| HealthError::not_found(format!("No hay pasos registrados el {}.", date_label(date))))?;
            let summary = format!("Eliminar los pasos del {} ({}).", date_label(date), steps_label(*steps));
            Ok(change(vec![StoreOp::DeleteSteps(date.trim().to_string())], summary))
        }
        HealthMutation::SetSleep { date, duration } => {
            let (date, minutes) = validate_sleep(date, duration, today).map_err(HealthError::fields)?;
            let summary = match data.sleep.get(&date) {
                Some(before) => format!("Cambiar el sueño del {}: {} → {}.", date_label(&date), sleep_label(*before), sleep_label(minutes)),
                None => format!("Registrar {} de sueño el {}.", sleep_label(minutes), date_label(&date)),
            };
            Ok(change(vec![StoreOp::PutSleep { date, minutes }], summary))
        }
        HealthMutation::DeleteSleep { date } => {
            let minutes = data.sleep.get(date.trim()).ok_or_else(|| HealthError::not_found(format!("No hay sueño registrado el {}.", date_label(date))))?;
            let summary = format!("Eliminar el sueño del {} ({}).", date_label(date), sleep_label(*minutes));
            Ok(change(vec![StoreOp::DeleteSleep(date.trim().to_string())], summary))
        }
        HealthMutation::SaveMeal { id, input } => {
            let (date, category, name, nutrition) = validate_meal(input, today).map_err(HealthError::fields)?;
            let before = match id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
                Some(id) => Some(data.meals.iter().find(|meal| meal.id == id).ok_or_else(|| HealthError::not_found("Esa comida ya no existe."))?),
                None => None,
            };
            let meal = Meal {
                id: before.map(|meal| meal.id.clone()).unwrap_or_else(|| ids.meal_id.clone()),
                date,
                category,
                name,
                nutrition,
                created_at_ms: before.map(|meal| meal.created_at_ms).unwrap_or(ids.now_ms),
                recipe_id: input.recipe_id.clone().or_else(|| before.and_then(|meal| meal.recipe_id.clone())),
            };
            let summary = format!(
                "{} «{}» en {} del {}: {} kcal, proteínas {} g, carbohidratos {} g, grasas {} g, fibra {} g.",
                if before.is_some() { "Cambiar" } else { "Agregar" },
                meal.name,
                meal.category.label().to_lowercase(),
                date_label(&meal.date),
                fmt(meal.nutrition.kcal, 0),
                fmt(meal.nutrition.protein_g, 1),
                fmt(meal.nutrition.carbs_g, 1),
                fmt(meal.nutrition.fat_g, 1),
                fmt(meal.nutrition.fiber_g, 1)
            );
            let meal_id = Some(meal.id.clone());
            Ok(Change { ops: vec![StoreOp::PutMeal(meal)], summary, meal_id })
        }
        HealthMutation::DeleteMeal { id } => {
            let meal = data.meals.iter().find(|meal| meal.id == id.trim()).ok_or_else(|| HealthError::not_found("Esa comida ya no existe."))?;
            Ok(change(
                vec![StoreOp::DeleteMeal(meal.id.clone())],
                format!("Eliminar «{}» ({} kcal) de {} del {}.", meal.name, fmt(meal.nutrition.kcal, 0), meal.category.label().to_lowercase(), date_label(&meal.date)),
            ))
        }
        HealthMutation::ImportWatch { days } => import_watch(data, days, today),
        HealthMutation::SetActivity { activity } => {
            let mut profile = data.profile.clone().ok_or_else(|| HealthError::validation("Primero completá tu perfil de Salud."))?;
            let label = super::activity_label(*activity).ok_or_else(|| HealthError::validation("Elegí un nivel de actividad de la lista."))?;
            profile.activity = *activity;
            Ok(change(vec![StoreOp::PutProfile(profile)], format!("Cambiar el nivel de actividad a «{label}».")))
        }
        HealthMutation::SetMeasuredExpenditure { enabled } => {
            let mut profile = data.profile.clone().ok_or_else(|| HealthError::validation("Primero completá tu perfil de Salud."))?;
            profile.use_measured_expenditure = *enabled;
            let summary = if *enabled {
                "Usar el gasto medido con tus comidas y tu peso en lugar del calculado con la fórmula."
            } else {
                "Volver al gasto calculado con la fórmula (metabolismo basal × actividad)."
            };
            Ok(change(vec![StoreOp::PutProfile(profile)], summary.to_string()))
        }
    }
}

/// Lo que trae el reloj: cada dato de cada día reemplaza el guardado y lo
/// que no llega no se toca. Un dato fuera de rango se descarta (el reloj
/// puede traer lecturas raras) en lugar de rechazar toda la importación.
/// La grasa y el metabolismo de la báscula del reloj son una medición de
/// ese día, con el peso del día (o el último) y lo que ya tuviera.
fn import_watch(data: &HealthData, days: &[WatchDay], today: Date) -> Result<Change, HealthError> {
    if days.len() > super::WATCH_IMPORT_MAX_DAYS {
        return Err(HealthError::validation(format!("Se importan hasta {} días por vez.", super::WATCH_IMPORT_MAX_DAYS)));
    }
    let oldest = super::days_ago(today, super::WATCH_IMPORT_OLDEST_DAYS);
    let height = data.profile.as_ref().map(|profile| profile.height_cm);
    let mut ops = Vec::new();
    let (mut steps_days, mut sleep_days, mut heart_days, mut weight_days, mut body_days) = (0, 0, 0, 0, 0);
    for day in days {
        let Some(date) = parse_date(day.date.trim()).filter(|date| *date <= today && *date >= oldest) else { continue };
        let key = date.to_string();
        if let Some(Ok((date, steps))) = day.steps.filter(|steps| *steps > 0).map(|steps| validate_steps(&key, &steps.to_string(), today)) {
            if data.steps.get(&date) != Some(&steps) {
                ops.push(StoreOp::PutSteps { date, steps });
                steps_days += 1;
            }
        }
        if let Some(Ok((date, minutes))) = day.sleep_minutes.filter(|minutes| *minutes > 0).map(|minutes| validate_sleep(&key, &format!("{minutes} min"), today)) {
            if data.sleep.get(&date) != Some(&minutes) {
                ops.push(StoreOp::PutSleep { date, minutes });
                sleep_days += 1;
            }
        }
        if let Some(bpm) = day.sleep_bpm.filter(|bpm| (25.0..=150.0).contains(bpm)).map(super::round1) {
            if data.sleep_bpm.get(&key) != Some(&bpm) {
                ops.push(StoreOp::PutSleepBpm { date: key.clone(), bpm });
                heart_days += 1;
            }
        }
        let weight = day.weight_kg.and_then(|kg| validate_weight(&key, Some(kg), today).ok());
        if let Some(entry) = &weight {
            if !data.weights.iter().any(|stored| stored.date == entry.date && (stored.kg - entry.kg).abs() < 0.05) {
                ops.push(StoreOp::PutWeight(entry.clone()));
                weight_days += 1;
            }
        }
        let mut values = std::collections::BTreeMap::new();
        if let Some(fat) = day.body_fat_pct {
            values.insert("grasaPct".to_string(), fat);
        }
        if let Some(bmr) = day.bmr_kcal {
            values.insert("metabolismo".to_string(), bmr);
        }
        if values.is_empty() {
            continue;
        }
        let stored = data.measurements.iter().find(|measurement| measurement.date == key);
        let kg = weight
            .as_ref()
            .map(|entry| entry.kg)
            .or_else(|| stored.map(|measurement| measurement.weight))
            .or_else(|| data.weights.iter().filter(|entry| entry.date <= key).last().map(|entry| entry.kg));
        let Some(kg) = kg else { continue };
        let mut merged = stored.map(|measurement| measurement.values.clone()).unwrap_or_default();
        // Lo que se calcula del peso o de la grasa se vuelve a calcular.
        let weight_changed = stored.map_or(true, |measurement| (measurement.weight - kg).abs() >= 0.05);
        let derived: &[&str] = if weight_changed {
            &["imc", "grasaKg", "pesoSinGrasa", "aguaKg", "musculoKg", "mmeKg"]
        } else if values.contains_key("grasaPct") {
            &["grasaKg", "pesoSinGrasa"]
        } else {
            &[]
        };
        for key in derived {
            merged.remove(*key);
        }
        merged.extend(values);
        let input = super::MeasurementInput { date: key.clone(), weight: Some(kg), values: merged };
        if let Ok(measurement) = validate_measurement(&input, height, today) {
            if stored != Some(&measurement) {
                ops.push(StoreOp::PutMeasurement(measurement));
                body_days += 1;
            }
        }
    }
    let parts = [(steps_days, "pasos"), (sleep_days, "sueño"), (heart_days, "pulso dormido"), (weight_days, "peso"), (body_days, "composición")]
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("{what} de {count} {}", if *count == 1 { "día" } else { "días" }))
        .collect::<Vec<_>>();
    let summary = if parts.is_empty() { "Sin datos nuevos del reloj.".to_string() } else { format!("Datos del reloj: {}.", parts.join(", ")) };
    Ok(Change { ops, summary, meal_id: None })
}

/// Lo que hace falta para armar un plan: perfil, peso, gasto y objetivo.
pub struct PlanBasis {
    pub current_kg: f64,
    pub target_kg: f64,
    pub tdee: f64,
    pub bmr: f64,
}

pub fn plan_basis(data: &HealthData, today: Date) -> Result<PlanBasis, HealthError> {
    let vitals = vitals(data, today);
    let (Some(current_kg), Some(tdee), Some(bmr)) = (vitals.current_kg, vitals.tdee, vitals.bmr) else {
        return Err(HealthError::validation("Para armar un plan hacen falta tu perfil (nacimiento, sexo, altura, actividad) y tu peso."));
    };
    let target_kg = data.objective.target_kg.ok_or_else(|| HealthError::validation("Fijá primero tu peso objetivo."))?;
    Ok(PlanBasis { current_kg, target_kg, tdee, bmr })
}

pub fn calculated_plan(data: &HealthData, today: Date) -> Result<Plan, HealthError> {
    let basis = plan_basis(data, today)?;
    let sex = data.profile.as_ref().map(|profile| profile.sex).unwrap_or(super::Sex::Male);
    Ok(calculate_plan(
        PlanInput { current_kg: basis.current_kg, target_kg: basis.target_kg, pace: data.objective.pace, tdee: basis.tdee, bmr: basis.bmr, sex },
        today,
    ))
}

/// El resumen de un plan de la IA, para la confirmación.
pub fn ai_plan_summary(plan: &Plan) -> String {
    let mut text = format!(
        "Guardar el plan con IA: {} kcal por día, proteínas {} g, carbohidratos {} g, grasas {} g, fibra {} g.",
        fmt(plan.kcal, 0),
        fmt(plan.protein_g, 0),
        fmt(plan.carbs_g, 0),
        fmt(plan.fat_g, 0),
        fmt(plan.fiber_g, 0)
    );
    if plan.weeks > 0 {
        text.push_str(&format!(" Tiempo estimado: {} semanas.", plan.weeks));
    }
    if !plan.summary.is_empty() {
        text.push(' ');
        text.push_str(&plan.summary);
    }
    for note in &plan.notes {
        text.push(' ');
        text.push_str(note);
    }
    text
}

/// Cambio de peso en un período, para la IA: «-3,6 kg».
pub fn weight_change(data: &HealthData, from: Date) -> Option<String> {
    let series = data.weights.iter().filter(|entry| parse_date(&entry.date).is_some_and(|date| date >= from)).collect::<Vec<_>>();
    (series.len() > 1).then(|| format!("{} kg", signed(series[series.len() - 1].kg - series[0].kg, 1)))
}
