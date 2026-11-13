//! Herramientas de la IA sobre Salud. Leen los argumentos y los convierten
//! en los mismos cambios que manda la pantalla, y arman lo que lee el modelo.
//! Cada usuario ve y cambia solo sus propios datos.

use jiff::civil::Date;
use serde_json::{json, Map, Value};

use super::change::{weight_change, HealthError};
use super::reference::{bmi_state, state};
use super::{
    active_plan, activity_label, days_ago, fold, meals_of, parse_date, parse_number, total, vitals, HealthData, HealthMutation, MacroKey,
    MealCategory, MealInput, MeasurementInput, ProfileInput, METRICS,
};

pub const HEALTH_READ_TOOLS: [&str; 2] = ["get_health_summary", "list_health_records"];
pub const HEALTH_WRITE_TOOLS: [&str; 11] = [
    "save_health_profile",
    "log_weight",
    "log_steps",
    "log_sleep",
    "save_body_measurement",
    "set_weight_goal",
    "set_health_plan",
    "log_water",
    "log_meal",
    "update_meal",
    "delete_health_record",
];
/// Registros que devuelve `list_health_records` como máximo.
const MAX_LISTED: usize = 400;

pub fn is_health_tool(name: &str) -> bool {
    HEALTH_READ_TOOLS.contains(&name) || is_health_write_tool(name)
}

pub fn is_health_write_tool(name: &str) -> bool {
    HEALTH_WRITE_TOOLS.contains(&name)
}

/// Qué hace una llamada de escritura.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolAction {
    Change(HealthMutation),
    /// Plan pedido a la IA con el objetivo guardado.
    AiPlan,
    /// Lo que comió la persona: se busca su receta en Recetas (o se crea) y
    /// se carga en Salud por la cantidad que comió.
    Meal(MealRequest),
}

/// Una comida contada por la persona o vista en una foto.
#[derive(Debug, Clone, PartialEq)]
pub struct MealRequest {
    /// Fecha, categoría, nombre del plato y, si los dijo, sus valores.
    pub input: MealInput,
    pub photo: Option<usize>,
    /// La receta que nombró (id o nombre exacto).
    pub recipe: Option<String>,
    pub portion: super::Portion,
    /// Para crear la receta: descripción e ingredientes con pesos.
    pub description: String,
    pub ingredients: Vec<String>,
}

fn invalid(message: impl Into<String>) -> HealthError {
    HealthError::validation(message)
}

fn text(arguments: &Value, name: &str) -> Option<String> {
    match arguments.get(name)? {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn number(arguments: &Value, name: &str) -> Result<Option<f64>, HealthError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => Ok(value.as_f64()),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => parse_number(value).map(Some).ok_or_else(|| invalid(format!("{name} tiene que ser un número."))),
        Some(_) => Err(invalid(format!("{name} tiene que ser un número."))),
    }
}

fn date_or_today(arguments: &Value, today: Date) -> String {
    text(arguments, "date").unwrap_or_else(|| today.to_string())
}

/// Un nivel de actividad por factor o por nombre («moderado»).
fn activity(arguments: &Value) -> Result<Option<f64>, HealthError> {
    if let Ok(Some(factor)) = number(arguments, "activity") {
        return Ok(Some(factor));
    }
    let Some(words) = text(arguments, "activity") else { return Ok(None) };
    let words = fold(&words);
    let factor = if words.contains("muy alto") || words.contains("doble") {
        1.9
    } else if words.contains("sedentari") {
        1.2
    } else if words.contains("ligero") || words.contains("leve") {
        1.375
    } else if words.contains("moderad") {
        1.55
    } else if words.contains("alto") || words.contains("intens") {
        1.725
    } else {
        return Err(invalid("activity: usá sedentario, ligero, moderado, alto o muy alto."));
    };
    Ok(Some(factor))
}

fn meal_nutrition(arguments: &Value, input: &mut MealInput) -> Result<(), HealthError> {
    for (name, slot) in [
        ("kcal", &mut input.kcal),
        ("proteinG", &mut input.protein_g),
        ("carbsG", &mut input.carbs_g),
        ("fatG", &mut input.fat_g),
        ("fiberG", &mut input.fiber_g),
    ] {
        if let Some(value) = number(arguments, name)? {
            *slot = Some(value);
        }
    }
    Ok(())
}

/// La comida que nombra una llamada: por id, o por descripción exacta (y
/// fecha si la da).
/// Candidates an error lists, so the next call names the meal by its id.
const MAX_MEAL_CANDIDATES: usize = 8;

fn meal_candidates(meals: &[&super::Meal]) -> String {
    meals
        .iter()
        .take(MAX_MEAL_CANDIDATES)
        .map(|meal| format!("{} ({}, {}) {} — {} kcal", meal.id, meal.date, meal.category.id(), meal.name, super::fmt(meal.nutrition.kcal, 0)))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The meal of an edit or a deletion: by id; by description, exact or the
/// only one that contains it (or is contained, without the portion that
/// follows the dish); without `meal`, the last one logged (that day, with
/// `date`). Without a single match, the error lists candidates with their
/// id instead of leaving the agent to search.
fn resolve_meal<'a>(data: &'a HealthData, arguments: &Value) -> Result<&'a super::Meal, HealthError> {
    let date = text(arguments, "date");
    let on_date = |meal: &&super::Meal| date.as_deref().is_none_or(|date| meal.date == date);
    let Some(reference) = text(arguments, "meal") else {
        return data
            .meals
            .iter()
            .filter(on_date)
            .max_by_key(|meal| meal.created_at_ms)
            .ok_or_else(|| HealthError::not_found("No hay comidas cargadas para ese día."));
    };
    if let Some(meal) = data.meals.iter().find(|meal| meal.id == reference) {
        return Ok(meal);
    }
    let wanted = fold(&reference);
    let exact = data.meals.iter().filter(on_date).filter(|meal| fold(&meal.name) == wanted).collect::<Vec<_>>();
    let found = if exact.is_empty() {
        data.meals
            .iter()
            .filter(on_date)
            .filter(|meal| {
                let name = fold(&meal.name);
                name.contains(&wanted) || wanted.contains(&name)
            })
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match found.as_slice() {
        [meal] => Ok(meal),
        [] => {
            let mut recent = data.meals.iter().filter(on_date).collect::<Vec<_>>();
            recent.sort_by_key(|meal| std::cmp::Reverse(meal.created_at_ms));
            let day = date.as_deref().map(|date| format!(" el {date}")).unwrap_or_default();
            Err(HealthError::not_found(if recent.is_empty() {
                format!("No hay comidas cargadas{day}.")
            } else {
                format!("No hay una comida «{reference}»{day}. Las últimas cargadas: {}. Repetí el pedido con meal = el id que corresponda.", meal_candidates(&recent))
            }))
        }
        several => Err(invalid(format!(
            "Hay {} comidas que coinciden con «{reference}»: {}. Repetí el pedido con meal = el id que corresponda.",
            several.len(),
            meal_candidates(several)
        ))),
    }
}

pub fn tool_action(name: &str, arguments: &Value, data: &HealthData, today: Date, hour: u32) -> Result<ToolAction, HealthError> {
    let change = |mutation| Ok(ToolAction::Change(mutation));
    match name {
        "save_health_profile" => {
            let stored = data.profile.as_ref();
            let input = ProfileInput {
                birth_date: text(arguments, "birthDate").or_else(|| stored.map(|profile| profile.birth_date.clone())).unwrap_or_default(),
                sex: text(arguments, "sex").or_else(|| stored.map(|profile| profile.sex.id().to_string())).unwrap_or_default(),
                height_cm: number(arguments, "heightCm")?.or(stored.map(|profile| profile.height_cm)),
                activity: activity(arguments)?.or(stored.map(|profile| profile.activity)),
                weight_kg: number(arguments, "weightKg")?,
                // Sin el dato, la meta elegida queda como estaba.
                water_goal_liters: number(arguments, "waterGoalLiters")?
                    .or(stored.and_then(|profile| profile.water_goal_ml).map(|ml| ml as f64 / 1000.0)),
            };
            change(HealthMutation::SaveProfile { input })
        }
        "log_weight" => change(HealthMutation::AddWeight { date: date_or_today(arguments, today), kg: number(arguments, "kg")? }),
        "log_steps" => {
            let steps = text(arguments, "steps").ok_or_else(|| invalid("Falta steps: la cantidad de pasos del día."))?;
            change(HealthMutation::SetSteps { date: date_or_today(arguments, today), steps })
        }
        "log_sleep" => {
            // Horas (7,5), minutos o una duración escrita («7:30», «7 h 30»).
            let duration = match (number(arguments, "hours")?, number(arguments, "minutes")?, text(arguments, "duration")) {
                (Some(hours), _, _) => format!("{} min", (hours * 60.0).round()),
                (None, Some(minutes), _) => format!("{} min", minutes.round()),
                (None, None, Some(duration)) => duration,
                (None, None, None) => return Err(invalid("Falta hours (o minutes, o duration como «7:30»): cuánto durmió.")),
            };
            change(HealthMutation::SetSleep { date: date_or_today(arguments, today), duration })
        }
        "save_body_measurement" => {
            let mut values = std::collections::BTreeMap::new();
            let nested = arguments.get("values").and_then(Value::as_object);
            for metric in METRICS.iter() {
                let value = match nested.and_then(|values| values.get(metric.key)) {
                    Some(value) => number(&json!({ "v": value }), "v")?,
                    None => number(arguments, metric.key)?,
                };
                if let Some(value) = value {
                    values.insert(metric.key.to_string(), value);
                }
            }
            if let Some(unknown) = nested.and_then(|values| values.keys().find(|key| super::metric_def(key).is_none())) {
                return Err(invalid(format!(
                    "values.{unknown} no es un valor de la balanza. Usá: {}.",
                    METRICS.iter().map(|metric| metric.key).collect::<Vec<_>>().join(", ")
                )));
            }
            change(HealthMutation::SaveMeasurement {
                input: MeasurementInput { date: date_or_today(arguments, today), weight: number(arguments, "weight")?, values },
            })
        }
        "set_weight_goal" => {
            let target_kg = match arguments.get("targetKg") {
                Some(Value::Null) => None,
                None => data.objective.target_kg,
                Some(_) => number(arguments, "targetKg")?,
            };
            let pace = number(arguments, "pace")?.or(Some(data.objective.pace));
            change(HealthMutation::SetObjective { target_kg, pace })
        }
        "set_health_plan" => match text(arguments, "mode").map(|mode| fold(&mode)).as_deref() {
            Some("ai") | Some("ia") => Ok(ToolAction::AiPlan),
            Some("calculated") | Some("calculado") => change(HealthMutation::CalculatePlan),
            Some("none") | Some("ninguno") | Some("mantenimiento") => change(HealthMutation::ClearPlan),
            _ => Err(invalid("mode tiene que ser ai (plan con IA), calculated (sin IA) o none (quitar el plan).")),
        },
        "log_water" => {
            let ml = number(arguments, "ml")?.ok_or_else(|| invalid("Falta ml: los mililitros de agua."))?.round() as i64;
            let date = date_or_today(arguments, today);
            match text(arguments, "mode").map(|mode| fold(&mode)).as_deref() {
                None | Some("add") | Some("sumar") => change(HealthMutation::AddWater { date, delta_ml: ml }),
                Some("set") | Some("fijar") => change(HealthMutation::SetWater { date, ml }),
                Some(_) => Err(invalid("mode tiene que ser add (sumar, o restar con ml negativo) o set (fijar el total del día).")),
            }
        }
        "log_meal" => {
            let mut input = MealInput {
                date: date_or_today(arguments, today),
                category: text(arguments, "category").unwrap_or_else(|| MealCategory::for_hour(hour).id().to_string()),
                name: text(arguments, "name").unwrap_or_default(),
                ..MealInput::default()
            };
            meal_nutrition(arguments, &mut input)?;
            let photo = number(arguments, "photoFromMessage")?.filter(|number| *number >= 1.0).map(|number| number.round() as usize);
            if input.name.is_empty() && photo.is_none() && text(arguments, "recipe").is_none() {
                return Err(invalid("Falta name: qué comió (o photoFromMessage con la foto del plato)."));
            }
            let portion = match (number(arguments, "servings")?, number(arguments, "grams")?, text(arguments, "portionNote")) {
                (_, _, Some(note)) => super::Portion::Note(note),
                (Some(servings), _, _) if (0.05..=20.0).contains(&servings) => super::Portion::Servings(servings),
                (Some(_), _, _) => return Err(invalid("servings va de 0,05 a 20 porciones.")),
                (None, Some(grams), _) if (1.0..=5000.0).contains(&grams) => super::Portion::Grams(grams),
                (None, Some(_), _) => return Err(invalid("grams va de 1 a 5000.")),
                (None, None, None) => super::Portion::One,
            };
            let ingredients = match arguments.get("ingredients") {
                Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                Some(Value::String(text)) => text.lines().map(str::to_string).collect(),
                _ => Vec::new(),
            };
            Ok(ToolAction::Meal(MealRequest {
                input,
                photo,
                recipe: text(arguments, "recipe"),
                portion,
                description: text(arguments, "description").unwrap_or_default(),
                ingredients,
            }))
        }
        "update_meal" => {
            let meal = resolve_meal(data, arguments)?;
            let mut input = MealInput {
                date: text(arguments, "newDate").unwrap_or_else(|| meal.date.clone()),
                category: text(arguments, "category").unwrap_or_else(|| meal.category.id().to_string()),
                name: text(arguments, "name").unwrap_or_else(|| meal.name.clone()),
                kcal: Some(meal.nutrition.kcal),
                protein_g: Some(meal.nutrition.protein_g),
                carbs_g: Some(meal.nutrition.carbs_g),
                fat_g: Some(meal.nutrition.fat_g),
                fiber_g: Some(meal.nutrition.fiber_g),
                recipe_id: meal.recipe_id.clone(),
            };
            meal_nutrition(arguments, &mut input)?;
            change(HealthMutation::SaveMeal { id: Some(meal.id.clone()), input })
        }
        "delete_health_record" => match text(arguments, "kind").map(|kind| fold(&kind)).as_deref() {
            Some("weight") | Some("peso") => {
                change(HealthMutation::DeleteWeight { date: text(arguments, "date").ok_or_else(|| invalid("Falta date del peso a eliminar."))? })
            }
            Some("measurement") | Some("medicion") => change(HealthMutation::DeleteMeasurement {
                date: text(arguments, "date").ok_or_else(|| invalid("Falta date de la medición a eliminar."))?,
            }),
            Some("meal") | Some("comida") => change(HealthMutation::DeleteMeal { id: resolve_meal(data, arguments)?.id.clone() }),
            Some("steps") | Some("pasos") => {
                change(HealthMutation::DeleteSteps { date: text(arguments, "date").ok_or_else(|| invalid("Falta date de los pasos a eliminar."))? })
            }
            Some("sleep") | Some("sueno") => {
                change(HealthMutation::DeleteSleep { date: text(arguments, "date").ok_or_else(|| invalid("Falta date del sueño a eliminar."))? })
            }
            _ => Err(invalid("kind tiene que ser weight, measurement, meal, steps o sleep.")),
        },
        _ => Err(invalid(format!("{name} no es una herramienta de Salud."))),
    }
}

fn round(value: f64) -> Value {
    json!((value * 10.0).round() / 10.0)
}

/// El estado de Salud de hoy para el modelo.
pub fn summary_view(data: &HealthData, today: Date) -> Value {
    let vitals = vitals(data, today);
    let Some(profile) = &data.profile else {
        return json!({
            "profile": null,
            "message": "No hay perfil de Salud: pedí nacimiento, sexo biológico, altura, actividad y peso, y guardalo con save_health_profile.",
            "weights": data.weights.len(),
        });
    };
    let plan = active_plan(data, &vitals);
    let key = today.to_string();
    let meals = meals_of(data, &key);
    let eaten = total(&meals);
    let last = data.measurements.last();
    json!({
        "today": key,
        "profile": {
            "birthDate": profile.birth_date,
            "age": vitals.age,
            "sex": profile.sex.label(),
            "heightCm": profile.height_cm,
            "activity": activity_label(profile.activity),
        },
        "weightKg": vitals.current_kg,
        "weightChange30Days": weight_change(data, days_ago(today, 30)),
        "bmi": vitals.bmi.map(|bmi| json!({ "value": round(bmi), "state": bmi_state(bmi).0 })),
        "bmrKcal": vitals.bmr.map(f64::round),
        "bmrSource": if vitals.bmr_from_scale { "balanza" } else { "Mifflin-St Jeor" },
        "dailyExpenditureKcal": vitals.tdee,
        "dailyExpenditureSource": if vitals.tdee_measured { "medido con comidas y peso" } else { "fórmula: metabolismo basal × actividad" },
        "formulaExpenditureKcal": vitals.tdee_formula,
        "measuredExpenditure": match &vitals.expenditure {
            super::ExpenditureStatus::Ready(measured) => json!({
                "kcal": measured.kcal,
                "intakeKcal": measured.intake,
                "kgPerWeek": (measured.kg_per_week * 100.0).round() / 100.0,
                "loggedDays": measured.logged_days,
                "inUse": vitals.tdee_measured,
            }),
            super::ExpenditureStatus::Missing(reason) => json!({ "missing": reason }),
        },
        "activitySuggestion": super::activity_suggestion(data, today).map(|suggestion| json!({
            "level": activity_label(suggestion.factor),
            "differsFromProfile": (suggestion.factor - profile.activity).abs() > 1e-6,
            "stepsAveragePerDay": suggestion.steps_average,
            "trainingDaysPerWeek": suggestion.exercise_days_per_week.map(|days| (days * 10.0).round() / 10.0),
        })),
        "sleepingHeartRate": super::sleep_heart(&data.sleep_bpm, today).map(|heart| json!({
            "lastNightBpm": heart.bpm,
            "twoWeekAverageBpm": heart.baseline.map(f64::round),
            "elevated": heart.elevated(),
        })),
        "objective": { "targetKg": data.objective.target_kg, "paceKgPerWeek": data.objective.pace },
        "plan": plan.map(|plan| json!({
            "kind": if data.plan.is_some() { format!("{:?}", plan.source).to_lowercase() } else { "maintenance".to_string() },
            "kcal": plan.kcal,
            "proteinG": plan.protein_g,
            "carbsG": plan.carbs_g,
            "fatG": plan.fat_g,
            "fiberG": plan.fiber_g,
            "weeks": plan.weeks,
            "summary": plan.summary,
            "notes": plan.notes,
            "recommendations": plan.recommendations,
        })),
        "todayFood": {
            "kcal": eaten.kcal,
            "remainingKcal": plan.map(|plan| plan.kcal - eaten.kcal),
            "macrosG": MacroKey::ALL.iter().map(|key| (key.id().to_string(), round(eaten.get(*key)))).collect::<Map<_, _>>(),
            "meals": meals.iter().map(|meal| json!({ "id": meal.id, "category": meal.category.id(), "name": meal.name, "kcal": meal.nutrition.kcal })).collect::<Vec<_>>(),
        },
        "waterTodayMl": data.water.get(&key).copied().unwrap_or_default(),
        "stepsToday": data.steps.get(&key),
        "stepsGoal": super::STEPS_GOAL,
        "sleepLastNightMinutes": data.sleep.get(&key),
        "sleepRecommendedHours": [super::SLEEP_RANGE_MINUTES.0 / 60, super::SLEEP_RANGE_MINUTES.1 / 60],
        "last7Days": (0..7).rev().map(|back| days_ago(today, back).to_string()).map(|date| json!({
            "date": date,
            "steps": data.steps.get(&date),
            "sleepMinutes": data.sleep.get(&date),
        })).collect::<Vec<_>>(),
        "waterGoalMl": vitals.water_goal_ml,
        "lastMeasurement": last.map(|measurement| json!({
            "date": measurement.date,
            "weightKg": measurement.weight,
            "values": measurement.values.iter().map(|(key, value)| {
                let status = state(key, *value, profile.sex, vitals.age).map(|(label, _)| label);
                (key.clone(), json!({ "value": value, "state": status }))
            }).collect::<Map<_, _>>(),
        })),
        "measurements": data.measurements.len(),
    })
}

/// Registros de un tipo entre dos fechas (los últimos 30 días si faltan).
pub fn records_view(data: &HealthData, arguments: &Value, today: Date) -> Result<Value, HealthError> {
    let from = text(arguments, "from").map(|from| parse_date(&from).ok_or_else(|| invalid("from tiene que ser YYYY-MM-DD."))).transpose()?.unwrap_or(days_ago(today, 30));
    let to = text(arguments, "to").map(|to| parse_date(&to).ok_or_else(|| invalid("to tiene que ser YYYY-MM-DD."))).transpose()?.unwrap_or(today);
    let within = |date: &str| parse_date(date).is_some_and(|date| date >= from && date <= to);
    let records: Vec<Value> = match text(arguments, "kind").map(|kind| fold(&kind)).as_deref() {
        Some("weights") | Some("weight") | Some("pesos") => {
            data.weights.iter().filter(|entry| within(&entry.date)).map(|entry| json!({ "date": entry.date, "kg": entry.kg })).collect()
        }
        Some("measurements") | Some("measurement") | Some("mediciones") => data
            .measurements
            .iter()
            .filter(|measurement| within(&measurement.date))
            .map(|measurement| json!({ "date": measurement.date, "weightKg": measurement.weight, "values": measurement.values }))
            .collect(),
        Some("meals") | Some("meal") | Some("comidas") => data
            .meals
            .iter()
            .filter(|meal| within(&meal.date))
            .map(|meal| {
                json!({
                    "id": meal.id,
                    "date": meal.date,
                    "category": meal.category.id(),
                    "name": meal.name,
                    "kcal": meal.nutrition.kcal,
                    "proteinG": meal.nutrition.protein_g,
                    "carbsG": meal.nutrition.carbs_g,
                    "fatG": meal.nutrition.fat_g,
                    "fiberG": meal.nutrition.fiber_g,
                    "recipeId": meal.recipe_id,
                })
            })
            .collect(),
        Some("water") | Some("agua") => data.water.iter().filter(|(date, _)| within(date)).map(|(date, ml)| json!({ "date": date, "ml": ml })).collect(),
        Some("steps") | Some("pasos") => data.steps.iter().filter(|(date, _)| within(date)).map(|(date, steps)| json!({ "date": date, "steps": steps })).collect(),
        Some("sleep") | Some("sueno") => data
            .sleep
            .iter()
            .filter(|(date, _)| within(date))
            .map(|(date, minutes)| json!({ "date": date, "minutes": minutes, "label": super::sleep_label(*minutes) }))
            .collect(),
        _ => return Err(invalid("kind tiene que ser weights, measurements, meals, water, steps o sleep.")),
    };
    let truncated = records.len() > MAX_LISTED;
    Ok(json!({
        "from": from.to_string(),
        "to": to.to_string(),
        "count": records.len(),
        "truncated": truncated,
        "records": records.into_iter().take(MAX_LISTED).collect::<Vec<_>>(),
    }))
}
