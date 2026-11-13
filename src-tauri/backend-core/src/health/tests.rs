use std::collections::BTreeMap;

use jiff::civil::{date, Date};
use serde_json::json;

use super::ai::{json_object, parse_meal, parse_plan};
use super::change::{plan_change, HealthErrorCode, NewIds, StoreOp};
use super::dashboard::{build_dashboard, DashboardQuery, MarkerKind, WeightRange};
use super::reference::{state, Tone};
use super::tools::{records_view, summary_view, tool_action, ToolAction};
use super::*;

const TODAY: Date = date(2026, 9, 28);

fn ids() -> NewIds {
    NewIds { meal_id: "meal-new".into(), now_ms: 1_000 }
}

fn profile(sex: Sex, age: i16, height: f64, activity: f64) -> Profile {
    Profile { birth_date: format!("{}-03-15", 2026 - age), sex, height_cm: height, activity, water_goal_ml: None }
}

fn meal(id: &str, date: &str, category: MealCategory, name: &str, kcal: f64, created: i64) -> Meal {
    Meal { id: id.into(), date: date.into(), category, name: name.into(), nutrition: Nutrition { kcal, protein_g: 20.0, carbs_g: 30.0, fat_g: 10.0, fiber_g: 4.0 }, created_at_ms: created, recipe_id: None }
}

/// El escenario «Sobrepeso» del canvas: 34 años, 178 cm, de 86 a 82,4 kg.
fn overweight() -> HealthData {
    let mut values = BTreeMap::new();
    values.insert("grasaPct".to_string(), 22.0);
    values.insert("grasaKg".to_string(), 18.1);
    values.insert("musculoKg".to_string(), 61.0);
    values.insert("huesosKg".to_string(), 3.2);
    values.insert("grasaVisceral".to_string(), 10.0);
    values.insert("edadMetabolica".to_string(), 38.0);
    let mut before = values.clone();
    before.insert("grasaPct".to_string(), 23.0);
    let mut water = BTreeMap::new();
    water.insert("2026-09-28".to_string(), 1_250);
    water.insert("2026-09-25".to_string(), 3_200);
    HealthData {
        profile: Some(profile(Sex::Male, 34, 178.0, 1.375)),
        weights: vec![
            WeightEntry { date: "2026-07-01".into(), kg: 86.0 },
            WeightEntry { date: "2026-08-15".into(), kg: 84.1 },
            WeightEntry { date: "2026-09-28".into(), kg: 82.4 },
        ],
        measurements: vec![
            Measurement { date: "2026-08-15".into(), weight: 84.1, values: before },
            Measurement { date: "2026-09-28".into(), weight: 82.4, values },
        ],
        objective: Objective { target_kg: Some(76.0), pace: 0.5 },
        plan: None,
        water,
        steps: [("2026-09-24", 6_200), ("2026-09-26", 9_800), ("2026-09-27", 11_250), ("2026-09-28", 4_310)]
            .into_iter()
            .map(|(date, steps)| (date.to_string(), steps))
            .collect(),
        sleep: [("2026-09-26", 400), ("2026-09-27", 465), ("2026-09-28", 390)].into_iter().map(|(date, minutes)| (date.to_string(), minutes)).collect(),
        meals: vec![
            meal("m1", "2026-09-28", MealCategory::Desayuno, "Café con leche y tostadas", 320.0, 1),
            meal("m2", "2026-09-28", MealCategory::Almuerzo, "Milanesa de pollo con ensalada", 520.0, 2),
            meal("m3", "2026-09-27", MealCategory::Almuerzo, "Fideos con bolognesa", 640.0, 3),
            meal("m4", "2026-09-26", MealCategory::Almuerzo, "milanesa de pollo con ensalada", 500.0, 0),
        ],
    }
}

#[test]
fn numbers_and_dates_read_as_in_argentina() {
    assert_eq!((fmt(1150.0, 0), fmt(82.4, 1), fmt(0.25, 2), fmt(-3.6, 1), fmt(-0.04, 1)), ("1.150".into(), "82,4".into(), "0,25".into(), "-3,6".into(), "0".into()));
    assert_eq!((signed(1.2, 1), signed(-3.6, 1), signed(0.0, 1)), ("+1,2".into(), "-3,6".into(), "0".into()));
    assert_eq!((parse_number("82,4"), parse_number("1.150"), parse_number("70 kg"), parse_number("x")), (Some(82.4), Some(1150.0), Some(70.0), None));
    assert_eq!((day_label(TODAY, TODAY), day_label(date(2026, 9, 27), TODAY), day_label(date(2026, 9, 21), TODAY)), ("Hoy".into(), "Ayer".into(), "lunes 21 sept".into()));
    assert_eq!((short_date(TODAY), long_date(TODAY)), ("28 sept 2026".into(), "28 de septiembre de 2026".into()));
    assert_eq!((age("1992-03-15", TODAY), age("1992-10-01", TODAY)), (Some(34), Some(33)));
    assert_eq!((MealCategory::for_hour(8), MealCategory::for_hour(13), MealCategory::for_hour(17), MealCategory::for_hour(22)), (MealCategory::Desayuno, MealCategory::Almuerzo, MealCategory::Merienda, MealCategory::Cena));
}

#[test]
fn the_calculated_plan_follows_the_pace_the_ceiling_and_the_floor() {
    let data = overweight();
    let vitals = vitals(&data, TODAY);
    assert_eq!((vitals.age, vitals.current_kg, vitals.bmr, vitals.tdee), (Some(34), Some(82.4), Some(1771.5), Some(2436.0)));
    assert_eq!(vitals.water_goal_ml, Some(3050));
    let plan = change::calculated_plan(&data, TODAY).expect("plan");
    assert_eq!((plan.kcal, plan.protein_g, plan.fat_g, plan.fiber_g, plan.carbs_g, plan.weeks), (1886.0, 137.0, 57.0, 26.0, 206.0, 13));
    assert_eq!(plan.summary, "Déficit de 550 kcal por día sobre tu gasto estimado.");
    assert!(plan.notes.is_empty());

    // 128 kg y 1 kg por semana: el déficit dejaría 1443 kcal, debajo del basal.
    let severe = calculate_plan(PlanInput { current_kg: 128.0, target_kg: 95.0, pace: 1.0, tdee: 2543.0, bmr: 2118.75, sex: Sex::Male }, TODAY);
    assert_eq!(severe.kcal, 2119.0);
    assert!(severe.notes[0].contains("2.119 kcal"));
    // Un ritmo mayor al 1 % del peso se recorta; un superávit no pasa de 500.
    let light = calculate_plan(PlanInput { current_kg: 40.0, target_kg: 48.0, pace: 1.0, tdee: 1800.0, bmr: 1100.0, sex: Sex::Female }, TODAY);
    assert!(light.notes[0].contains("0,4 kg por semana"));
    assert_eq!(light.kcal, 2240.0);
    assert!(light.summary.starts_with("Superávit"));
    let keep = calculate_plan(PlanInput { current_kg: 70.0, target_kg: 70.2, pace: 0.5, tdee: 2600.0, bmr: 1700.0, sex: Sex::Male }, TODAY);
    assert_eq!((keep.kcal, keep.weeks), (2600.0, 0));
    // Sin balanza, el basal es Mifflin; con balanza, el de la balanza.
    let mut scale = overweight();
    scale.measurements[1].values.insert("metabolismo".into(), 1850.0);
    let from_scale = super::vitals(&scale, TODAY);
    assert_eq!((from_scale.bmr, from_scale.bmr_from_scale), (Some(1850.0), true));
}

#[test]
fn forms_are_validated_field_by_field() {
    let young = ProfileInput { birth_date: "2015-01-01".into(), sex: "x".into(), height_cm: Some(90.0), activity: Some(1.3), weight_kg: Some(10.0), water_goal_liters: None };
    let fields = validate_profile(&young, TODAY).unwrap_err().into_iter().map(|error| error.field).collect::<Vec<_>>();
    assert_eq!(fields, ["birthDate", "sex", "heightCm", "activity", "weightKg"]);
    assert!(validate_weight("2026-09-29", Some(80.0), TODAY).is_err());
    assert!(validate_weight("2026-09-28", Some(401.0), TODAY).is_err());
    let measurement = MeasurementInput { date: "2026-09-28".into(), weight: Some(80.0), values: [("grasaPct".to_string(), 25.0), ("aguaPct".to_string(), 0.0)].into() };
    let complete = validate_measurement(&measurement, Some(180.0), TODAY).expect("measurement");
    // El cero es «vacío»; lo derivable se completa.
    assert_eq!((complete.value("aguaPct"), complete.value("grasaKg"), complete.value("pesoSinGrasa"), complete.value("imc")), (None, Some(20.0), Some(60.0), Some(24.7)));
    let unknown = MeasurementInput { values: [("colesterol".to_string(), 180.0)].into(), ..measurement };
    assert_eq!(validate_measurement(&unknown, None, TODAY).unwrap_err()[0].field, "colesterol");
    let empty = MealInput { date: "2026-09-28".into(), category: "cena".into(), name: "Tarta".into(), ..MealInput::default() };
    assert_eq!(validate_meal(&empty, TODAY).unwrap_err()[0].field, "kcal");
    let macros = MealInput { protein_g: Some(20.0), carbs_g: Some(30.0), fat_g: Some(10.0), ..empty.clone() };
    assert_eq!(validate_meal(&macros, TODAY).expect("meal").3.kcal, 290.0);
    assert!(validate_objective(Some(70.0), Some(0.3)).is_err());
    assert!(validate_water("2026-09-28", 25_000, TODAY).is_err());
}

#[test]
fn changes_write_what_the_summary_says() {
    let data = overweight();
    let input = ProfileInput { birth_date: "1992-03-15".into(), sex: "masculino".into(), height_cm: Some(178.0), activity: Some(1.55), weight_kg: Some(82.0), water_goal_liters: None };
    let saved = plan_change(&data, &HealthMutation::SaveProfile { input: input.clone() }, TODAY, &ids()).expect("profile");
    assert_eq!(saved.ops.len(), 2);
    assert!(saved.summary.contains("Moderado") && saved.summary.contains("82 kg"));
    let same_weight = plan_change(&data, &HealthMutation::SaveProfile { input: ProfileInput { weight_kg: Some(82.4), ..input } }, TODAY, &ids()).expect("profile");
    assert_eq!(same_weight.ops.len(), 1);

    let replace = plan_change(&data, &HealthMutation::AddWeight { date: "2026-09-28".into(), kg: Some(82.0) }, TODAY, &ids()).expect("weight");
    assert_eq!(replace.summary, "Cambiar el peso del 28 sept 2026: 82,4 kg → 82 kg.");

    let scale = MeasurementInput { date: "2026-09-27".into(), weight: Some(82.6), values: [("grasaPct".to_string(), 21.5)].into() };
    let measured = plan_change(&data, &HealthMutation::SaveMeasurement { input: scale }, TODAY, &ids()).expect("measurement");
    assert!(matches!(&measured.ops[1], StoreOp::PutWeight(entry) if entry.kg == 82.6));
    assert!(measured.summary.starts_with("Registrar la medición de la balanza del 27 sept 2026: 82,6 kg"));

    let less = plan_change(&data, &HealthMutation::AddWater { date: "2026-09-28".into(), delta_ml: -2_000 }, TODAY, &ids()).expect("water");
    assert_eq!(less.ops, vec![StoreOp::PutWater { date: "2026-09-28".into(), ml: 0 }]);

    let new_meal = MealInput { date: "2026-09-28".into(), category: "merienda".into(), name: "Mate con medialunas".into(), kcal: Some(290.0), ..MealInput::default() };
    let added = plan_change(&data, &HealthMutation::SaveMeal { id: None, input: new_meal.clone() }, TODAY, &ids()).expect("meal");
    assert_eq!(added.meal_id.as_deref(), Some("meal-new"));
    let edited = plan_change(&data, &HealthMutation::SaveMeal { id: Some("m1".into()), input: new_meal }, TODAY, &ids()).expect("edit");
    assert!(matches!(&edited.ops[0], StoreOp::PutMeal(meal) if meal.id == "m1" && meal.created_at_ms == 1));
    assert_eq!(plan_change(&data, &HealthMutation::DeleteMeal { id: "nada".into() }, TODAY, &ids()).unwrap_err().code, HealthErrorCode::NotFound);
    assert_eq!(plan_change(&data, &HealthMutation::ClearPlan, TODAY, &ids()).unwrap_err().code, HealthErrorCode::NotFound);
    let no_goal = HealthData { objective: Objective::default(), ..overweight() };
    assert!(plan_change(&no_goal, &HealthMutation::CalculatePlan, TODAY, &ids()).unwrap_err().message.contains("objetivo"));
}

#[test]
fn the_dashboard_arrives_ready_to_show() {
    let data = overweight();
    let dashboard = build_dashboard(&data, &DashboardQuery::default(), TODAY, 13);
    assert_eq!(dashboard.header.chips, ["34 años", "178 cm", "82,4 kg"]);
    assert_eq!(dashboard.header.bmi.as_ref().map(|chip| chip.label.as_str()), Some("IMC 26, sobrepeso"));

    let bmi = dashboard.bmi.expect("bmi");
    assert_eq!(bmi.message, "Estás 3,5 kg por encima del rango normal para tu altura.");
    assert_eq!(bmi.bar.segments.iter().filter(|segment| segment.active).map(|segment| segment.label.as_str()).collect::<Vec<_>>(), ["Sobrepeso"]);
    assert_eq!(bmi.bar.markers.iter().map(|marker| marker.kind).collect::<Vec<_>>(), [MarkerKind::Goal, MarkerKind::Value]);
    assert_eq!(bmi.bar.ticks.iter().map(|tick| tick.label.as_str()).collect::<Vec<_>>(), ["18,5", "25", "30", "35", "40"]);
    assert_eq!(bmi.healthy_range_label, "Peso normal para tu altura: 58,6 a 78,9 kg");

    let weight = dashboard.weight;
    assert_eq!(weight.summary, "-3,6 kg en el período. Te faltan 6,4 kg para tu objetivo.");
    let chart = weight.chart.expect("chart");
    assert_eq!(chart.points.iter().map(|point| point.x).collect::<Vec<_>>(), [0.0, 0.5, 1.0]);
    assert_eq!(chart.goal.as_ref().map(|goal| goal.label.as_str()), Some("Objetivo 76 kg"));
    assert!(chart.y_ticks.iter().all(|tick| (0.0..=1.0).contains(&tick.at)));
    // En 30 días hay un solo peso: no alcanza para el gráfico.
    let month = build_dashboard(&data, &DashboardQuery { weight_range: WeightRange::Days30, ..DashboardQuery::default() }, TODAY, 13);
    assert!(month.weight.chart.is_none() && month.weight.summary == "Te faltan 6,4 kg para tu objetivo.");
    assert_eq!(weight.entries[0].kg_label, "82,4 kg");

    let objective = dashboard.objective;
    assert_eq!((objective.difference_label.as_deref(), objective.target_bmi_label.as_deref(), objective.can_plan), (Some("-6,4 kg"), Some("24"), true));

    let food = dashboard.food.expect("food");
    assert_eq!((food.day_label.as_str(), food.total_label.as_str(), food.target_label.as_str()), ("Hoy", "840", "de 2.436 kcal"));
    assert_eq!(food.next_date, None);
    assert_eq!(food.remaining_label, "Quedan 1.596 kcal");
    assert_eq!(food.macros[0].status, format!("Faltan {} g", fmt(food_plan_protein(&data) - 40.0, 0)));
    assert_eq!(food.groups.iter().map(|group| group.meals.len()).collect::<Vec<_>>(), [1, 0, 1, 0, 0]);
    assert!(food.footnote.contains("Mifflin-St Jeor") && food.footnote.contains("mantenimiento"));
    let yesterday = build_dashboard(&data, &DashboardQuery { food_date: Some("2026-09-27".into()), ..DashboardQuery::default() }, TODAY, 13).food.expect("food");
    assert_eq!((yesterday.day_label.as_str(), yesterday.next_date.as_deref()), ("Ayer", Some("2026-09-28")));

    let water = dashboard.water.expect("water");
    assert_eq!((water.percent_label.as_str(), water.liters_label.as_str(), water.goal_label.as_str()), ("41%", "1,25 L", "de 3,05 L"));
    assert_eq!(water.week.len(), 7);
    assert!(water.week[3].met && water.week[6].date == "2026-09-28");

    let composition = dashboard.composition.expect("composition");
    assert!(composition.subtitle.ends_with("28 de septiembre de 2026, comparada con la anterior"));
    assert_eq!(composition.parts.iter().map(|part| part.key.as_str()).collect::<Vec<_>>(), ["fat", "muscle", "bone"]);
    let fat = &composition.groups[0].metrics[0];
    assert_eq!((fat.value_label.as_str(), fat.delta_label.as_deref(), fat.state.as_ref().map(|chip| chip.label.as_str())), ("22", Some("-1"), Some("Alto")));
    let metabolic_age = composition.groups[3].metrics.iter().find(|metric| metric.key == "edadMetabolica").expect("age");
    assert_eq!(metabolic_age.state.as_ref().map(|chip| chip.tone), Some(Tone::Warn));

    // Recientes: sin repetir la misma comida escrita distinto.
    let lunches = dashboard.meal_form.recent.iter().filter(|meal| meal.category == "almuerzo").map(|meal| meal.name.as_str()).collect::<Vec<_>>();
    assert_eq!(lunches, ["Milanesa de pollo con ensalada", "Fideos con bolognesa"]);
    assert_eq!(dashboard.meal_form.suggested_category, "almuerzo");
    assert_eq!(dashboard.measurement_fields.iter().map(|group| group.fields.len()).sum::<usize>(), METRICS.len());

    let empty = build_dashboard(&HealthData::default(), &DashboardQuery::default(), TODAY, 9);
    assert!(!empty.has_profile && empty.bmi.is_none() && empty.food.is_none() && empty.water.is_none() && empty.composition.is_none());
}

fn food_plan_protein(data: &HealthData) -> f64 {
    super::vitals(data, TODAY).maintenance.expect("maintenance").protein_g
}

#[test]
fn the_phone_cards_summarize_today_and_the_last_month() {
    let mut data = overweight();
    // Las calorías de hoy no siguen al día que muestra la alimentación.
    let yesterday = build_dashboard(&data, &DashboardQuery { food_date: Some("2026-09-27".into()), ..DashboardQuery::default() }, TODAY, 13);
    let today = yesterday.today_calories.expect("today");
    assert_eq!((yesterday.food.expect("food").total_label.as_str(), today.total_label.as_str(), today.target_label.as_str()), ("640", "840", "de 2.436 kcal"));
    assert_eq!((today.over, today.remaining_label.as_str()), (false, "Quedan 1.596 kcal"));
    assert_eq!(today.macros.iter().map(|indicator| format!("{} {}", indicator.value_label, indicator.target_label)).collect::<Vec<_>>()[0], format!("40 / {} g", fmt(food_plan_protein(&data), 0)));

    // Un solo peso en el mes: sin cambio ni línea, pero con lo que falta.
    let trend = |data: &HealthData| build_dashboard(data, &DashboardQuery::default(), TODAY, 13).weight_trend;
    let lone = trend(&data);
    assert_eq!((lone.change_label, lone.goal_label.as_deref(), lone.line), (None, Some("Objetivo 76 kg, faltan 6,4"), None));

    // De 81,9 a 84,1 kg (medio kilo de aire), en una caja de 100 con y hacia abajo.
    data.weights.insert(2, WeightEntry { date: "2026-09-10".into(), kg: 83.6 });
    let pair = trend(&data);
    assert_eq!((pair.change_label.as_deref(), pair.line.as_deref()), (Some("-1,2 kg en 30 días"), Some("M0,22.73L100,77.27")));
    data.weights.insert(3, WeightEntry { date: "2026-09-20".into(), kg: 83.0 });
    assert_eq!(trend(&data).line.as_deref(), Some("M0,22.73C16.67,31.82 33.33,40.91 50,50C66.67,59.09 83.33,68.18 100,77.27"));
    // Con subidas y bajadas la curva no se pasa de la caja.
    data.weights.insert(4, WeightEntry { date: "2026-09-25".into(), kg: 83.9 });
    let wavy = trend(&data).line.expect("line");
    let numbers = wavy.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter(|text| !text.is_empty()).map(|text| text.parse::<f64>().expect("number")).collect::<Vec<_>>();
    assert_eq!(wavy.matches('C').count(), 3);
    assert!(numbers.iter().all(|value| (0.0..=100.0).contains(value)), "{wavy}");

    // Sin objetivo, o ya en él, no hay «faltan».
    data.objective.target_kg = Some(82.4);
    assert_eq!(trend(&data).goal_label, None);
    let empty = build_dashboard(&HealthData::default(), &DashboardQuery::default(), TODAY, 13);
    assert!(empty.today_calories.is_none() && empty.weight_trend == super::dashboard::WeightTrend { change_label: None, goal_label: None, line: None });
}

/// Los tableros de las pruebas de Vitest (`src/modules/health/components/__fixtures__`):
#[test]
fn steps_and_sleep_are_read_as_written_and_validated() {
    assert_eq!(validate_steps("2026-09-28", "8.432", TODAY), Ok(("2026-09-28".to_string(), 8_432)));
    assert_eq!(validate_steps("2026-09-28", "12000", TODAY).map(|(_, steps)| steps), Ok(12_000));
    assert!(validate_steps("2026-09-28", "", TODAY).is_err());
    assert!(validate_steps("2026-09-28", "200000", TODAY).is_err());
    assert!(validate_steps("2026-09-29", "100", TODAY).is_err(), "no future days");
    for (text, minutes) in [("7,5", 450), ("7.5", 450), ("7:30", 450), ("7 h 30", 450), ("7h30m", 450), ("7 horas y media", 450), ("8", 480), ("450 min", 450), ("45 minutos", 45)] {
        assert_eq!(parse_sleep_minutes(text), Some(minutes), "{text}");
    }
    assert_eq!(parse_sleep_minutes("mucho"), None);
    assert_eq!(parse_sleep_minutes("7:75"), None);
    assert!(validate_sleep("2026-09-28", "30", TODAY).is_err(), "30 hours");
    assert!(validate_sleep("2026-09-28", "0", TODAY).is_err());
    assert_eq!(sleep_label(450), "7 h 30 min");
    assert_eq!(sleep_label(480), "8 h");
    assert_eq!(sleep_label(45), "45 min");
    assert_eq!(steps_label(8_432), "8.432 pasos");
}

#[test]
fn steps_and_sleep_changes_write_and_delete_one_day() {
    let data = overweight();
    let set = plan_change(&data, &HealthMutation::SetSteps { date: "2026-09-28".into(), steps: "5.000".into() }, TODAY, &ids()).expect("steps");
    assert_eq!(set.ops, vec![StoreOp::PutSteps { date: "2026-09-28".into(), steps: 5_000 }]);
    assert!(set.summary.contains("4.310 pasos → 5.000 pasos"), "{}", set.summary);
    let slept = plan_change(&data, &HealthMutation::SetSleep { date: "2026-09-25".into(), duration: "8:15".into() }, TODAY, &ids()).expect("sleep");
    assert_eq!(slept.ops, vec![StoreOp::PutSleep { date: "2026-09-25".into(), minutes: 495 }]);
    assert!(slept.summary.starts_with("Registrar 8 h 15 min de sueño"), "{}", slept.summary);
    let deleted = plan_change(&data, &HealthMutation::DeleteSleep { date: "2026-09-27".into() }, TODAY, &ids()).expect("delete");
    assert_eq!(deleted.ops, vec![StoreOp::DeleteSleep("2026-09-27".into())]);
    let missing = plan_change(&data, &HealthMutation::DeleteSteps { date: "2026-09-01".into() }, TODAY, &ids()).expect_err("missing");
    assert_eq!(missing.code, HealthErrorCode::NotFound);
}

#[test]
fn steps_and_sleep_panels_show_today_the_week_and_the_history() {
    let dashboard = build_dashboard(&overweight(), &DashboardQuery { steps_range: WeightRange::Days30, ..DashboardQuery::default() }, TODAY, 13);
    let steps = &dashboard.steps;
    assert_eq!((steps.today_label.as_str(), steps.today_caption.as_str()), ("4.310", "Hoy"));
    assert!(steps.summary.starts_with("Promedio de la semana: 7.890 pasos."), "{}", steps.summary);
    assert_eq!(steps.entries.first().map(|row| row.value_label.as_str()), Some("4.310 pasos"));
    let chart = steps.chart.as_ref().expect("chart");
    assert_eq!(chart.points.len(), 4);
    assert!(chart.y_ticks.first().is_some_and(|tick| tick.at == 0.0 && tick.label == "0"));
    assert!(chart.points.iter().all(|point| (0.0..=1.0).contains(&point.y)));
    assert_eq!(chart.goal.as_ref().map(|goal| goal.label.as_str()), Some("Meta 8.000"));
    let sleep = &dashboard.sleep;
    assert_eq!((sleep.today_label.as_str(), sleep.today_caption.as_str()), ("6 h 30 min", "Anoche"));
    assert!(sleep.summary.contains("Promedio de la semana: 6 h 58 min, menos de lo recomendado.") && sleep.summary.contains("Recomendado: 7 a 9\u{a0}h."), "{}", sleep.summary);
    assert_eq!(sleep.chart.as_ref().map(|chart| chart.points[0].value_label.as_str()), Some("6 h 40 min"));
    let empty = build_dashboard(&HealthData::default(), &DashboardQuery::default(), TODAY, 13);
    assert!(empty.steps.chart.is_none() && empty.steps.today_label == "—" && empty.sleep.entries.is_empty());
}

#[test]
fn the_assistant_logs_steps_and_sleep_and_reads_them_back() {
    let data = overweight();
    let action = |name: &str, arguments: serde_json::Value| tool_action(name, &arguments, &data, TODAY, 13);
    assert_eq!(action("log_steps", json!({ "steps": 9000 })), Ok(ToolAction::Change(HealthMutation::SetSteps { date: "2026-09-28".into(), steps: "9000".into() })));
    assert_eq!(action("log_sleep", json!({ "hours": 7.5 })), Ok(ToolAction::Change(HealthMutation::SetSleep { date: "2026-09-28".into(), duration: "450 min".into() })));
    assert_eq!(action("log_sleep", json!({ "duration": "6:45", "date": "2026-09-27" })), Ok(ToolAction::Change(HealthMutation::SetSleep { date: "2026-09-27".into(), duration: "6:45".into() })));
    assert!(action("log_sleep", json!({})).is_err());
    assert_eq!(action("delete_health_record", json!({ "kind": "sueño", "date": "2026-09-27" })), Ok(ToolAction::Change(HealthMutation::DeleteSleep { date: "2026-09-27".into() })));
    let summary = summary_view(&data, TODAY);
    assert_eq!(summary["stepsToday"], 4_310);
    assert_eq!(summary["sleepLastNightMinutes"], 390);
    assert_eq!(summary["last7Days"].as_array().map(Vec::len), Some(7));
    let sleep = records_view(&data, &json!({ "kind": "sleep" }), TODAY).expect("records");
    assert_eq!(sleep["count"], 3);
    assert_eq!(sleep["records"][0]["label"], "6 h 40 min");
    let parsed = plan_change(&data, &HealthMutation::SetSleep { date: "2026-09-28".into(), duration: "450 min".into() }, TODAY, &ids()).expect("sleep");
    assert_eq!(parsed.ops, vec![StoreOp::PutSleep { date: "2026-09-28".into(), minutes: 450 }]);
}

/// «Sobrepeso» con el plan calculado, y sin perfil.
/// `HEALTH_FIXTURES=<carpeta> cargo test -p notia-backend-core dump_dashboard_fixtures -- --ignored`.
#[test]
#[ignore]
fn dump_dashboard_fixtures() {
    let Ok(out) = std::env::var("HEALTH_FIXTURES") else { return };
    let out = std::path::Path::new(&out);
    let mut planned = overweight();
    planned.plan = change::calculated_plan(&planned, TODAY).ok();
    for (name, data) in [("dashboard.json", planned), ("dashboard-empty.json", HealthData::default())] {
        let dashboard = build_dashboard(&data, &DashboardQuery::default(), TODAY, 13);
        std::fs::write(out.join(name), serde_json::to_string_pretty(&dashboard).expect("json") + "\n").expect("write");
    }
}

#[test]
fn reference_states_follow_sex_and_age() {
    assert_eq!(state("grasaPct", 30.0, Sex::Female, None), Some(("Normal", Tone::Ok)));
    assert_eq!(state("grasaPct", 30.0, Sex::Male, None), Some(("Muy alto", Tone::High)));
    assert_eq!(state("edadMetabolica", 30.0, Sex::Male, Some(34)), Some(("Igual o menor a tu edad", Tone::Ok)));
    assert_eq!(state("huesosKg", 3.0, Sex::Male, None), None);
}

#[test]
fn ai_answers_are_read_and_bounded() {
    let answer = "```json\n{\"kcal\": 1300, \"proteina_g\": 140, \"carbohidratos_g\": \"150\", \"grasas_g\": 55, \"fibra_g\": 28, \"semanas_estimadas\": 14, \"resumen\": \"Déficit moderado.\", \"recomendaciones\": [\"Caminá\", \"\", \"Dormí 8 h\"], \"advertencia\": \"\"}\n```";
    let plan = parse_plan(answer, 1772.0, TODAY).expect("plan");
    assert_eq!((plan.kcal, plan.carbs_g, plan.source), (1772.0, 150.0, PlanSource::Ai));
    assert_eq!(plan.recommendations, ["Caminá", "Dormí 8 h"]);
    assert!(plan.notes[0].contains("1.772 kcal"));
    assert!(parse_plan("{\"kcal\": 2000}", 1500.0, TODAY).is_none());
    let (nutrition, name) = parse_meal("Listo: {\"nombre\": \"2 empanadas de carne\", \"kcal\": 540, \"proteina_g\": 22, \"carbohidratos_g\": 48, \"grasas_g\": 28.44}").expect("meal");
    assert_eq!((nutrition.kcal, nutrition.fat_g, nutrition.fiber_g, name.as_str()), (540.0, 28.4, 0.0, "2 empanadas de carne"));
    assert!(json_object("sin json").is_none());
}

#[test]
fn tools_read_arguments_into_the_same_changes() {
    let data = overweight();
    let action = tool_action("log_meal", &json!({ "name": "Guiso de lentejas", "photoFromMessage": 1 }), &data, TODAY, 13).expect("meal");
    let ToolAction::Meal(request) = action else { panic!("{action:?}") };
    assert_eq!((request.input.category.as_str(), request.input.date.as_str(), request.input.has_nutrition(), request.photo), ("almuerzo", "2026-09-28", false, Some(1)));
    assert_eq!(request.portion, Portion::One);
    let weighed = tool_action("log_meal", &json!({ "name": "Guiso de lentejas", "grams": "500", "ingredients": ["200 g de lentejas", "1 cebolla"] }), &data, TODAY, 13).expect("meal");
    let ToolAction::Meal(weighed) = weighed else { panic!() };
    assert_eq!((weighed.portion, weighed.ingredients.len()), (Portion::Grams(500.0), 2));
    let noted = tool_action("log_meal", &json!({ "recipe": "Guiso de lentejas", "servings": 2, "portionNote": "con doble porción de papas" }), &data, TODAY, 13).expect("meal");
    let ToolAction::Meal(noted) = noted else { panic!() };
    assert_eq!((noted.recipe.as_deref(), noted.portion), (Some("Guiso de lentejas"), Portion::Note("con doble porción de papas".into())));
    assert!(tool_action("log_meal", &json!({ "category": "cena" }), &data, TODAY, 13).is_err());
    assert!(tool_action("log_meal", &json!({ "name": "Guiso", "servings": 50 }), &data, TODAY, 13).is_err());
    assert_eq!(
        tool_action("set_weight_goal", &json!({ "targetKg": "75" }), &data, TODAY, 13).expect("goal"),
        ToolAction::Change(HealthMutation::SetObjective { target_kg: Some(75.0), pace: Some(0.5) })
    );
    assert_eq!(
        tool_action("set_weight_goal", &json!({ "targetKg": null }), &data, TODAY, 13).expect("goal"),
        ToolAction::Change(HealthMutation::SetObjective { target_kg: None, pace: Some(0.5) })
    );
    assert_eq!(tool_action("set_health_plan", &json!({ "mode": "IA" }), &data, TODAY, 13).expect("plan"), ToolAction::AiPlan);
    assert_eq!(
        tool_action("log_water", &json!({ "ml": 500 }), &data, TODAY, 13).expect("water"),
        ToolAction::Change(HealthMutation::AddWater { date: "2026-09-28".into(), delta_ml: 500 })
    );
    let profile = tool_action("save_health_profile", &json!({ "activity": "moderado" }), &data, TODAY, 13).expect("profile");
    let ToolAction::Change(HealthMutation::SaveProfile { input }) = profile else { panic!() };
    assert_eq!((input.activity, input.height_cm, input.sex.as_str()), (Some(1.55), Some(178.0), "M"));
    let measurement = tool_action("save_body_measurement", &json!({ "weight": 82.1, "values": { "grasaPct": "21,8" }, "grasaVisceral": 9 }), &data, TODAY, 13).expect("scale");
    let ToolAction::Change(HealthMutation::SaveMeasurement { input }) = measurement else { panic!() };
    assert_eq!((input.values.get("grasaPct"), input.values.get("grasaVisceral")), (Some(&21.8), Some(&9.0)));
    assert!(tool_action("save_body_measurement", &json!({ "weight": 80, "values": { "colesterol": 1 } }), &data, TODAY, 13).is_err());
    // Una comida por descripción (varias con el mismo nombre piden el id).
    assert_eq!(
        tool_action("delete_health_record", &json!({ "kind": "meal", "meal": "Fideos con bolognesa" }), &data, TODAY, 13).expect("delete"),
        ToolAction::Change(HealthMutation::DeleteMeal { id: "m3".into() })
    );
    assert!(tool_action("update_meal", &json!({ "meal": "milanesa de pollo con ensalada" }), &data, TODAY, 13).unwrap_err().message.contains("m2"));
    // Part of the description finds the only meal that has it; none lists the last ones.
    let ToolAction::Change(HealthMutation::SaveMeal { id: Some(id), input }) =
        tool_action("update_meal", &json!({ "meal": "bolognesa", "category": "cena" }), &data, TODAY, 13).expect("by part")
    else {
        panic!()
    };
    assert_eq!((id.as_str(), input.category.as_str()), ("m3", "cena"));
    let missing = tool_action("update_meal", &json!({ "meal": "sushi" }), &data, TODAY, 13).unwrap_err();
    assert!(missing.message.contains("Las últimas cargadas: ") && missing.message.contains("m3 (") && missing.message.contains(" kcal"), "{}", missing.message);
    // Without meal, the last one logged.
    let ToolAction::Change(HealthMutation::SaveMeal { id: Some(last), .. }) =
        tool_action("update_meal", &json!({ "category": "merienda" }), &data, TODAY, 13).expect("last")
    else {
        panic!()
    };
    let newest = data.meals.iter().max_by_key(|meal| meal.created_at_ms).expect("meal");
    assert_eq!(last, newest.id);
    let update = tool_action("update_meal", &json!({ "meal": "m2", "kcal": 600 }), &data, TODAY, 13).expect("update");
    let ToolAction::Change(HealthMutation::SaveMeal { id: Some(id), input }) = update else { panic!() };
    assert_eq!((id.as_str(), input.kcal, input.protein_g), ("m2", Some(600.0), Some(20.0)));

    let summary = summary_view(&data, TODAY);
    assert_eq!(summary["todayFood"]["kcal"], json!(840.0));
    assert_eq!(summary["plan"]["kind"], json!("maintenance"));
    assert_eq!(summary["lastMeasurement"]["values"]["grasaPct"]["state"], json!("Alto"));
    assert!(summary_view(&HealthData::default(), TODAY)["message"].as_str().unwrap_or_default().contains("save_health_profile"));
    let meals = records_view(&data, &json!({ "kind": "comidas", "from": "2026-09-27" }), TODAY).expect("records");
    assert_eq!(meals["count"], json!(3));
    assert!(records_view(&data, &json!({ "kind": "otra" }), TODAY).is_err());
}

#[test]
fn a_meal_is_scaled_from_its_recipe() {
    let mut recipe = crate::recipes::Recipe {
        id: "r1".into(),
        name: "Guiso de lentejas".into(),
        meal: crate::recipes::MealTime::Lunch,
        minutes: Some(60),
        servings: Some(4),
        serving_grams: Some(350.0),
        description: String::new(),
        ingredients: vec!["400 g de lentejas".into()],
        steps: Vec::new(),
        nutrition: [("kcal".to_string(), 450.0), ("prot".to_string(), 24.0), ("carb".to_string(), 60.0), ("grasa".to_string(), 12.0), ("fibra".to_string(), 16.0)].into(),
        photo: None,
        created_at_ms: 0,
        updated_at_ms: 0,
        ai_reviewed: true,
        path: "recipes/Guiso de lentejas.md".into(),
    };
    let serving = recipe_serving(&recipe);
    assert_eq!((serving.kcal, serving.fiber_g), (450.0, 16.0));
    // 500 g of a 350 g serving.
    let factor = portion_factor(&recipe, &Portion::Grams(500.0)).expect("factor");
    let eaten = scale(serving, factor);
    assert_eq!((eaten.kcal, eaten.protein_g, eaten.carbs_g, eaten.fat_g), (643.0, 34.3, 85.7, 17.1));
    assert_eq!(portion_factor(&recipe, &Portion::Servings(1.5)), Some(1.5));
    assert_eq!(portion_factor(&recipe, &Portion::Note("con papas".into())), None);
    // A new recipe from the plate is one serving; a photo of an existing one goes to the AI.
    assert_eq!(eaten_factor(&recipe, &Portion::One, false), Some(1.0));
    assert_eq!(eaten_factor(&recipe, &Portion::One, true), None);
    assert_eq!(eaten_factor(&recipe, &Portion::Servings(2.0), true), Some(2.0));
    recipe.serving_grams = None;
    // Grams without the serving weight go to the AI.
    assert_eq!(portion_factor(&recipe, &Portion::Grams(500.0)), None);
    assert_eq!(portion_name("Guiso de lentejas", &Portion::Grams(500.0)), "Guiso de lentejas · 500 g");
    assert_eq!(portion_name("Guiso de lentejas", &Portion::Servings(1.5)), "Guiso de lentejas · 1,5 porciones");
    assert_eq!(portion_name("Guiso de lentejas", &Portion::Servings(1.0)), "Guiso de lentejas");
    assert_eq!(portion_name("Guiso", &Portion::Note("con doble papas".into())), "Guiso (con doble papas)");
    // The same amount always says how it was calculated; the AI says it estimated.
    assert_eq!(meal_basis("Guiso", 660.0, Some(1.0), ""), "Calculado: 1 porción de «Guiso» (660 kcal por porción).");
    assert_eq!(meal_basis("Guiso", 660.0, Some(1.25), ""), "Calculado: 1,25 porciones de «Guiso» (660 kcal por porción).");
    assert!(meal_basis("Guiso", 660.0, None, "sobras de la cena").starts_with("Estimado por la IA para «sobras de la cena»"));
    assert_eq!(recipe_meal_time(MealCategory::Merienda), crate::recipes::MealTime::Snack);

    // The AI answer for another amount: its macros, or its servings.
    let (system, user) = super::ai::portion_prompt(&recipe, "un plato grande, con doble porción de papas", true);
    assert!(system.contains("JSON") && user.contains("\"receta\": \"Guiso de lentejas\"") && user.contains("La foto de lo que comió"));
    let with_macros = super::ai::parse_portion("{\"porciones\": 1.8, \"kcal\": 840, \"proteina_g\": 40, \"carbohidratos_g\": 120, \"grasas_g\": 20.04, \"fibra_g\": 28}", serving).expect("portion");
    assert_eq!((with_macros.kcal, with_macros.fat_g), (840.0, 20.0));
    let with_servings = super::ai::parse_portion("{\"porciones\": 2}", serving).expect("portion");
    assert_eq!(with_servings.kcal, 900.0);
    assert!(super::ai::parse_portion("{\"porciones\": 90}", serving).is_none());

    // A meal keeps its recipe; an edit without one keeps the stored one.
    let data = HealthData { meals: vec![Meal { recipe_id: Some("r1".into()), ..meal("m1", "2026-09-28", MealCategory::Almuerzo, "Guiso", 450.0, 1) }], ..HealthData::default() };
    let input = MealInput { date: "2026-09-28".into(), category: "almuerzo".into(), name: "Guiso".into(), kcal: Some(500.0), ..MealInput::default() };
    let edited = plan_change(&data, &HealthMutation::SaveMeal { id: Some("m1".into()), input }, TODAY, &ids()).expect("edit");
    assert!(matches!(&edited.ops[0], StoreOp::PutMeal(meal) if meal.recipe_id.as_deref() == Some("r1")));
}

/// A water goal chosen in the profile replaces the calculated one: empty or
/// 0 goes back to it, out of range is refused, the cards say which one
/// rules, and the assistant keeps it when it changes something else.
#[test]
fn a_chosen_water_goal_replaces_the_calculated_one() {
    let mut data = overweight();
    let calculated = vitals(&data, TODAY);
    assert_eq!((calculated.water_goal_ml, calculated.water_goal_auto_ml), (Some(3050), Some(3050)));

    let input = ProfileInput { birth_date: "1992-03-15".into(), sex: "M".into(), height_cm: Some(178.0), activity: Some(1.375), weight_kg: None, water_goal_liters: Some(2.48) };
    let (chosen, _) = validate_profile(&input, TODAY).expect("profile");
    assert_eq!(chosen.water_goal_ml, Some(2500), "rounded to 50 ml");
    for automatic in [None, Some(0.0)] {
        let (profile, _) = validate_profile(&ProfileInput { water_goal_liters: automatic, ..input.clone() }, TODAY).expect("profile");
        assert_eq!(profile.water_goal_ml, None);
    }
    for wrong in [0.2, 6.5] {
        let errors = validate_profile(&ProfileInput { water_goal_liters: Some(wrong), ..input.clone() }, TODAY).expect_err("range");
        assert!(errors.iter().any(|error| error.field == "waterGoalLiters"), "{wrong}");
    }

    data.profile = Some(chosen);
    let ruling = vitals(&data, TODAY);
    assert_eq!((ruling.water_goal_ml, ruling.water_goal_auto_ml), (Some(2500), Some(3050)));
    let dashboard = build_dashboard(&data, &DashboardQuery::default(), TODAY, 13);
    let water = dashboard.water.expect("water");
    assert_eq!((water.goal_label.as_str(), water.basis_label.as_str()), ("de 2,5 L", "Meta elegida en tu perfil"));
    assert_eq!((dashboard.profile_form.water_goal_liters, dashboard.profile_form.water_goal_auto_label.as_deref()), (Some(2.5), Some("3,05 L")));

    let changed = tool_action("save_health_profile", &json!({ "activity": "alto" }), &data, TODAY, 13).expect("profile");
    let ToolAction::Change(HealthMutation::SaveProfile { input }) = changed else { panic!() };
    assert_eq!(input.water_goal_liters, Some(2.5));
    let reset = tool_action("save_health_profile", &json!({ "waterGoalLiters": 0 }), &data, TODAY, 13).expect("profile");
    let ToolAction::Change(HealthMutation::SaveProfile { input }) = reset else { panic!() };
    assert_eq!(validate_profile(&input, TODAY).expect("profile").0.water_goal_ml, None);
}
