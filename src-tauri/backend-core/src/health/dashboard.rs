//! El tablero de Salud listo para mostrar: textos, barras, el gráfico de peso
//! y los datos de los formularios. React no calcula nada.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use super::reference::{bmi_range, bmi_state, range_for, state, Range, Tone};
use super::{
    active_plan, activity_label, day_label, day_month, days_ago, fmt, long_date, meals_of, parse_date, shift_days, short_date,
    signed, sleep_label, steps_label, total, vitals, weekday_narrow, HealthData, MacroKey, MealCategory, MetricGroup, Nutrition, Plan, PlanSource, Sex,
    Vitals, ACTIVITIES, METRICS, PACES, SLEEP_RANGE_MINUTES, STEPS_GOAL, activity_suggestion, sleep_heart, ExpenditureStatus,
};

// ---------------------------------------------------------------------------
// Consulta
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeightRange {
    #[serde(rename = "30")]
    Days30,
    #[default]
    #[serde(rename = "90")]
    Days90,
    #[serde(rename = "365")]
    Year,
    #[serde(rename = "all")]
    All,
}

impl WeightRange {
    const ALL: [Self; 4] = [Self::Days30, Self::Days90, Self::Year, Self::All];

    fn id(self) -> &'static str {
        match self {
            Self::Days30 => "30",
            Self::Days90 => "90",
            Self::Year => "365",
            Self::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Days30 => "30 días",
            Self::Days90 => "90 días",
            Self::Year => "1 año",
            Self::All => "Todo",
        }
    }

    fn days(self) -> Option<i64> {
        match self {
            Self::Days30 => Some(30),
            Self::Days90 => Some(90),
            Self::Year => Some(365),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardQuery {
    /// Día de la alimentación; hoy si falta.
    pub food_date: Option<String>,
    #[serde(default)]
    pub weight_range: WeightRange,
    /// Medición elegida; la última si falta.
    pub measurement_date: Option<String>,
    #[serde(default)]
    pub steps_range: WeightRange,
    #[serde(default)]
    pub sleep_range: WeightRange,
}

// ---------------------------------------------------------------------------
// DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChip {
    pub label: String,
    pub tone: Tone,
}

fn chip(label: &str, tone: Tone) -> StateChip {
    StateChip { label: label.to_string(), tone }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSegment {
    pub label: String,
    pub tone: Tone,
    /// Ancho relativo del tramo.
    pub flex: f64,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkerKind {
    Value,
    Goal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarMarker {
    /// 0 a 100.
    pub position: f64,
    pub kind: MarkerKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarTick {
    pub position: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeBar {
    pub segments: Vec<BarSegment>,
    pub markers: Vec<BarMarker>,
    pub ticks: Vec<BarTick>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub chips: Vec<String>,
    pub bmi: Option<StateChip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalLegend {
    pub label: String,
    pub state: Option<StateChip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BmiPanel {
    pub value_label: String,
    pub state: StateChip,
    pub message: String,
    pub bar: RangeBar,
    pub today_label: String,
    pub goal: Option<GoalLegend>,
    pub healthy_range_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartPoint {
    /// 0 (izquierda) a 1.
    pub x: f64,
    /// 0 (abajo) a 1.
    pub y: f64,
    pub label: String,
    pub value_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartTick {
    pub at: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightChart {
    pub points: Vec<ChartPoint>,
    pub y_ticks: Vec<ChartTick>,
    pub x_ticks: Vec<ChartTick>,
    pub goal: Option<ChartTick>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightRow {
    pub date: String,
    pub date_label: String,
    pub kg_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightPanel {
    pub current_label: String,
    pub summary: String,
    pub ranges: Vec<Choice>,
    pub chart: Option<WeightChart>,
    pub empty_text: String,
    pub entries: Vec<WeightRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyRow {
    pub date: String,
    pub date_label: String,
    pub value_label: String,
}

/// Un registro diario con su histórico, como el peso: los pasos o el sueño.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPanel {
    /// Los pasos de hoy o el sueño de anoche; «—» sin registro.
    pub today_label: String,
    /// «Hoy», «Anoche».
    pub today_caption: String,
    /// El promedio de la última semana y la meta.
    pub summary: String,
    pub ranges: Vec<Choice>,
    pub chart: Option<WeightChart>,
    pub empty_text: String,
    pub entries: Vec<DailyRow>,
    /// El pulso dormido de anoche (solo en el sueño, si el reloj lo trajo).
    pub heart: Option<HeartView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartView {
    /// «Pulso dormido 54 lpm».
    pub label: String,
    /// Frente al promedio de las 2 semanas anteriores.
    pub detail: String,
    /// `ok`, `warn` o `muted` (sin promedio todavía).
    pub tone: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySuggestionView {
    pub value: String,
    pub label: String,
    /// De dónde sale: los pasos y los entrenamientos de 4 semanas.
    pub reason: String,
}

/// Lo que las últimas semanas dicen para ajustar el plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustView {
    pub activity_label: String,
    /// Solo si difiere del nivel del perfil.
    pub activity_suggestion: Option<ActivitySuggestionView>,
    /// «2.900 kcal»: metabolismo basal × actividad.
    pub formula_label: Option<String>,
    /// «2.680 kcal»: lo que comiste menos lo que explica el cambio de peso.
    pub measured_label: Option<String>,
    /// Cómo se midió, o qué falta para medirlo.
    pub measured_detail: String,
    pub using_measured: bool,
    pub can_use_measured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub kcal_label: String,
    pub source_label: String,
    pub weeks_label: Option<String>,
    pub summary: String,
    pub notes: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectivePanel {
    pub target_kg: Option<f64>,
    pub pace: f64,
    pub paces: Vec<Choice>,
    pub difference_label: Option<String>,
    pub target_bmi_label: Option<String>,
    pub target_state: Option<StateChip>,
    pub low_bmi_warning: bool,
    pub can_plan: bool,
    pub plan: Option<PlanView>,
    pub adjust: Option<AdjustView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroIndicator {
    pub key: String,
    pub label: String,
    pub value_label: String,
    pub target_label: String,
    pub percent_label: String,
    /// 0 a 100.
    pub progress: f64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroPill {
    pub key: String,
    pub short: String,
    pub label: String,
    pub value_label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MealRow {
    pub id: String,
    pub name: String,
    pub kcal_label: String,
    pub pills: Vec<MacroPill>,
    /// Los valores guardados, para editarla.
    pub values: RecentMeal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MealGroup {
    pub id: String,
    pub label: String,
    pub kcal_label: Option<String>,
    pub meals: Vec<MealRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoodPanel {
    pub date: String,
    pub day_label: String,
    pub previous_date: String,
    pub next_date: Option<String>,
    pub total_label: String,
    pub target_label: String,
    pub progress: f64,
    pub over: bool,
    pub remaining_label: String,
    pub stats: Vec<StatRow>,
    pub footnote: String,
    pub macros: Vec<MacroIndicator>,
    pub groups: Vec<MealGroup>,
}

/// Las calorías y los macros de un día frente al plan que rige.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaloriesSummary {
    pub total_label: String,
    pub target_label: String,
    /// 0 a 100.
    pub progress: f64,
    pub over: bool,
    pub remaining_label: String,
    pub macros: Vec<MacroIndicator>,
}

/// El peso de los últimos 30 días (la tarjeta «Peso» del celular), sin
/// importar el período que muestra el gráfico.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightTrend {
    /// «-1,2 kg en 30 días»; con dos pesos o más en el mes.
    pub change_label: Option<String>,
    /// «Objetivo 76 kg, faltan 6,4».
    pub goal_label: Option<String>,
    /// La línea del mes como trazo SVG en una caja de 100 × 100 (y hacia
    /// abajo) que se estira a la tarjeta; sin línea con menos de dos pesos.
    pub line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterDay {
    pub date: String,
    pub day_label: String,
    pub fill: f64,
    pub met: bool,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterPanel {
    pub ml: i64,
    /// 0 a 1.
    pub ratio: f64,
    pub percent_label: String,
    pub liters_label: String,
    pub goal_label: String,
    pub basis_label: String,
    pub aria_label: String,
    pub week: Vec<WaterDay>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyPart {
    pub key: String,
    pub label: String,
    pub detail: String,
    /// 0 a 100.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRow {
    pub key: String,
    pub label: String,
    pub value_label: String,
    pub unit: Option<String>,
    pub delta_label: Option<String>,
    pub bar: Option<RangeBar>,
    pub state: Option<StateChip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricGroupView {
    pub id: String,
    pub title: String,
    pub metrics: Vec<MetricRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositionPanel {
    pub date: String,
    pub subtitle: String,
    pub dates: Vec<Choice>,
    pub strip_title: String,
    pub parts: Vec<BodyPart>,
    pub groups: Vec<MetricGroupView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileForm {
    pub birth_date: String,
    pub sex: String,
    pub height_cm: Option<f64>,
    pub activity: f64,
    pub weight_kg: Option<f64>,
    pub activities: Vec<Choice>,
    /// La meta de agua elegida, en litros; vacía si se calcula.
    pub water_goal_liters: Option<f64>,
    /// La que se calcula con altura y peso («3,3 L»), si hay datos.
    pub water_goal_auto_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMeal {
    pub category: String,
    pub name: String,
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MealForm {
    pub suggested_category: String,
    pub categories: Vec<Choice>,
    /// Hasta 5 por categoría, las más recientes sin repetir.
    pub recent: Vec<RecentMeal>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementField {
    pub key: String,
    pub label: String,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementFieldGroup {
    pub id: String,
    pub title: String,
    pub fields: Vec<MeasurementField>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthDashboard {
    pub today: String,
    pub has_profile: bool,
    pub header: Header,
    pub bmi: Option<BmiPanel>,
    pub weight: WeightPanel,
    pub objective: ObjectivePanel,
    pub food: Option<FoodPanel>,
    /// Las calorías de hoy aunque la alimentación muestre otro día.
    pub today_calories: Option<CaloriesSummary>,
    pub weight_trend: WeightTrend,
    pub water: Option<WaterPanel>,
    pub steps: DailyPanel,
    pub sleep: DailyPanel,
    pub composition: Option<CompositionPanel>,
    pub profile_form: ProfileForm,
    pub meal_form: MealForm,
    pub measurement_fields: Vec<MeasurementFieldGroup>,
}

// ---------------------------------------------------------------------------
// Armado
// ---------------------------------------------------------------------------

pub fn build_dashboard(data: &HealthData, query: &DashboardQuery, today: Date, hour: u32) -> HealthDashboard {
    let vitals = vitals(data, today);
    let sex = data.profile.as_ref().map(|profile| profile.sex).unwrap_or(Sex::Male);
    HealthDashboard {
        today: today.to_string(),
        has_profile: data.profile.is_some(),
        header: header(data, &vitals),
        bmi: bmi_panel(data, &vitals),
        weight: weight_panel(data, &vitals, query.weight_range, today),
        objective: objective_panel(data, &vitals, today),
        food: food_panel(data, &vitals, query.food_date.as_deref(), today),
        today_calories: active_plan(data, &vitals).map(|plan| calories_summary(&total(&meals_of(data, &today.to_string())), plan)),
        weight_trend: weight_trend(data, &vitals, today),
        water: water_panel(data, &vitals, today),
        steps: steps_panel(data, query.steps_range, today),
        sleep: sleep_panel(data, query.sleep_range, today),
        composition: composition_panel(data, &vitals, sex, query.measurement_date.as_deref()),
        profile_form: profile_form(data, &vitals),
        meal_form: meal_form(data, hour),
        measurement_fields: measurement_fields(),
    }
}

fn header(data: &HealthData, vitals: &Vitals) -> Header {
    let Some(profile) = &data.profile else {
        return Header { chips: Vec::new(), bmi: None };
    };
    let mut chips = Vec::new();
    if let Some(age) = vitals.age {
        chips.push(format!("{age} años"));
    }
    chips.push(format!("{} cm", fmt(profile.height_cm, 0)));
    if let Some(kg) = vitals.current_kg {
        chips.push(format!("{} kg", fmt(kg, 1)));
    }
    let bmi = vitals.bmi.map(|bmi| {
        let (label, tone) = bmi_state(bmi);
        chip(&format!("IMC {}, {}", fmt(bmi, 1), label.to_lowercase()), tone)
    });
    Header { chips, bmi }
}

/// La barra de un rango con la posición del valor y otras marcas.
pub fn range_bar(range: &Range, value: Option<f64>, goal: Option<f64>, ticks: bool) -> RangeBar {
    let position = |x: f64| (((x - range.min) / (range.max - range.min)) * 100.0).clamp(0.0, 100.0);
    let mut pieces = Vec::new();
    let mut from = range.min;
    for segment in &range.segments {
        let to = segment.until.min(range.max);
        if to > from {
            pieces.push((segment, from, to));
        }
        from = from.max(to);
        if from >= range.max {
            break;
        }
    }
    let active = value.map(|value| pieces.iter().position(|(_, _, to)| value < *to).unwrap_or(pieces.len().saturating_sub(1)));
    let segments = pieces
        .iter()
        .enumerate()
        .map(|(index, (segment, from, to))| BarSegment {
            label: segment.label.to_string(),
            tone: segment.tone,
            flex: to - from,
            active: active == Some(index),
        })
        .collect();
    let mut markers = Vec::new();
    if let Some(goal) = goal {
        markers.push(BarMarker { position: position(goal), kind: MarkerKind::Goal });
    }
    if let Some(value) = value {
        markers.push(BarMarker { position: position(value), kind: MarkerKind::Value });
    }
    let ticks = if ticks {
        pieces[..pieces.len().saturating_sub(1)]
            .iter()
            .map(|(_, _, to)| BarTick { position: position(*to), label: fmt(*to, 1) })
            .collect()
    } else {
        Vec::new()
    };
    RangeBar { segments, markers, ticks }
}

fn bmi_panel(data: &HealthData, vitals: &Vitals) -> Option<BmiPanel> {
    let profile = data.profile.as_ref()?;
    let bmi = vitals.bmi?;
    let current = vitals.current_kg?;
    let (label, tone) = bmi_state(bmi);
    let squared = (profile.height_cm / 100.0).powi(2);
    let healthy_min = 18.5 * squared;
    let healthy_max = 24.9 * squared;
    let message = if current > healthy_max {
        format!("Estás {} kg por encima del rango normal para tu altura.", fmt(current - healthy_max, 1))
    } else if current < healthy_min {
        format!("Te faltan {} kg para entrar en el rango normal para tu altura.", fmt(healthy_min - current, 1))
    } else {
        "Estás dentro del rango normal para tu altura.".to_string()
    };
    let goal_bmi = data.objective.target_kg.and_then(|target| super::bmi(target, profile.height_cm));
    Some(BmiPanel {
        value_label: fmt(bmi, 1),
        state: chip(label, tone),
        message,
        bar: range_bar(&bmi_range(), Some(bmi), goal_bmi, true),
        today_label: format!("Hoy {}", fmt(bmi, 1)),
        goal: goal_bmi.map(|goal| {
            let (label, tone) = bmi_state(goal);
            GoalLegend { label: format!("Con tu objetivo {}", fmt(goal, 1)), state: Some(chip(label, tone)) }
        }),
        healthy_range_label: format!("Peso normal para tu altura: {} a {} kg", fmt(healthy_min, 1), fmt(healthy_max, 1)),
    })
}

/// Un paso «redondo» para el eje, con hasta 6 marcas.
fn axis_step(span: f64) -> f64 {
    [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0].into_iter().find(|step| span / step <= 6.0).unwrap_or(100.0)
}

fn weight_chart(points: &[(Date, f64)], goal: Option<f64>) -> Option<WeightChart> {
    if points.len() < 2 {
        return None;
    }
    let values = points.iter().map(|(_, kg)| *kg).chain(goal);
    let (low, high) = values.fold((f64::MAX, f64::MIN), |(low, high), kg| (low.min(kg), high.max(kg)));
    let (y_min, y_max) = ((low - 1.0).floor(), (high + 1.0).ceil());
    let span = (y_max - y_min).max(1.0);
    let y = |kg: f64| (kg - y_min) / span;
    let last = (points.len() - 1) as f64;
    let step = axis_step(span);
    let mut y_ticks = Vec::new();
    let mut tick = (y_min / step).ceil() * step;
    while tick <= y_max + 1e-9 {
        y_ticks.push(ChartTick { at: y(tick), label: fmt(tick, 1) });
        tick += step;
    }
    let tick_count = points.len().min(6);
    let mut x_ticks = Vec::new();
    for index in 0..tick_count {
        let point = if tick_count == 1 { 0 } else { (index as f64 * last / (tick_count - 1) as f64).round() as usize };
        x_ticks.push(ChartTick { at: point as f64 / last, label: day_month(points[point].0) });
    }
    x_ticks.dedup_by(|a, b| a.at == b.at);
    Some(WeightChart {
        points: points
            .iter()
            .enumerate()
            .map(|(index, (date, kg))| ChartPoint {
                x: index as f64 / last,
                y: y(*kg),
                label: short_date(*date),
                value_label: format!("{} kg", fmt(*kg, 1)),
            })
            .collect(),
        y_ticks,
        x_ticks,
        goal: goal.map(|goal| ChartTick { at: y(goal), label: format!("Objetivo {} kg", fmt(goal, 1)) }),
    })
}

fn weight_panel(data: &HealthData, vitals: &Vitals, range: WeightRange, today: Date) -> WeightPanel {
    let from = range.days().map(|days| days_ago(today, days));
    let series = data
        .weights
        .iter()
        .filter_map(|entry| parse_date(&entry.date).map(|date| (date, entry.kg)))
        .filter(|(date, _)| from.map_or(true, |from| *date >= from))
        .collect::<Vec<_>>();
    let mut summary = Vec::new();
    if series.len() > 1 {
        let change = series[series.len() - 1].1 - series[0].1;
        summary.push(format!("{} kg en el período.", signed(change, 1)));
    }
    if let (Some(target), Some(current)) = (data.objective.target_kg, vitals.current_kg) {
        let missing = current - target;
        if missing.abs() >= 0.1 {
            summary.push(format!("Te faltan {} kg para tu objetivo.", fmt(missing.abs(), 1)));
        }
    }
    WeightPanel {
        current_label: vitals.current_kg.map(|kg| fmt(kg, 1)).unwrap_or_else(|| "—".to_string()),
        summary: summary.join(" "),
        ranges: WeightRange::ALL
            .iter()
            .map(|option| Choice { value: option.id().to_string(), label: option.label().to_string(), selected: *option == range })
            .collect(),
        chart: weight_chart(&series, data.objective.target_kg),
        empty_text: "Cargá al menos dos pesos para ver tu evolución.".to_string(),
        entries: data
            .weights
            .iter()
            .rev()
            .map(|entry| WeightRow {
                date: entry.date.clone(),
                date_label: parse_date(&entry.date).map(short_date).unwrap_or_else(|| entry.date.clone()),
                kg_label: format!("{} kg", fmt(entry.kg, 1)),
            })
            .collect(),
    }
}

/// Un paso de eje redondo (1, 2, 2,5 o 5 por potencia de 10) con hasta seis
/// marcas en `span`.
fn nice_step(span: f64) -> f64 {
    let magnitude = 10f64.powf((span / 6.0).max(1e-9).log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().map(|factor| factor * magnitude).find(|step| span / step <= 6.0).unwrap_or(10.0 * magnitude)
}

/// El gráfico de una serie diaria (pasos, sueño) con el eje desde cero y la
/// meta como línea; necesita dos registros o más.
fn daily_chart(
    points: &[(Date, f64)],
    goal: Option<(f64, String)>,
    tick_label: impl Fn(f64) -> String,
    value_label: impl Fn(f64) -> String,
) -> Option<WeightChart> {
    if points.len() < 2 {
        return None;
    }
    let high = points.iter().map(|(_, value)| *value).chain(goal.as_ref().map(|(value, _)| *value)).fold(0.0, f64::max).max(1.0);
    let step = nice_step(high * 1.1);
    let top = ((high * 1.1) / step).ceil() * step;
    let y = |value: f64| value / top;
    let last = (points.len() - 1) as f64;
    let y_ticks = (0..=((top / step).round() as usize)).map(|index| index as f64 * step).map(|tick| ChartTick { at: y(tick), label: tick_label(tick) }).collect();
    let tick_count = points.len().min(6);
    let mut x_ticks = (0..tick_count)
        .map(|index| (index as f64 * last / (tick_count - 1) as f64).round() as usize)
        .map(|point| ChartTick { at: point as f64 / last, label: day_month(points[point].0) })
        .collect::<Vec<_>>();
    x_ticks.dedup_by(|a, b| a.at == b.at);
    Some(WeightChart {
        points: points
            .iter()
            .enumerate()
            .map(|(index, (date, value))| ChartPoint { x: index as f64 / last, y: y(*value), label: short_date(*date), value_label: value_label(*value) })
            .collect(),
        y_ticks,
        x_ticks,
        goal: goal.map(|(value, label)| ChartTick { at: y(value), label }),
    })
}

/// Los registros de una serie diaria dentro del período, en orden.
fn daily_series(values: &std::collections::BTreeMap<String, i64>, range: WeightRange, today: Date) -> Vec<(Date, i64)> {
    let from = range.days().map(|days| days_ago(today, days));
    values
        .iter()
        .filter_map(|(date, value)| parse_date(date).map(|date| (date, *value)))
        .filter(|(date, _)| from.map_or(true, |from| *date >= from))
        .collect()
}

/// El promedio de los días con registro de la última semana.
fn week_average(values: &std::collections::BTreeMap<String, i64>, today: Date) -> Option<f64> {
    let week = daily_series(values, WeightRange::Days30, today).into_iter().filter(|(date, _)| *date > days_ago(today, 7)).map(|(_, value)| value).collect::<Vec<_>>();
    (!week.is_empty()).then(|| week.iter().sum::<i64>() as f64 / week.len() as f64)
}

fn range_choices(range: WeightRange) -> Vec<Choice> {
    WeightRange::ALL.iter().map(|option| Choice { value: option.id().to_string(), label: option.label().to_string(), selected: *option == range }).collect()
}

fn daily_rows(values: &std::collections::BTreeMap<String, i64>, label: impl Fn(i64) -> String) -> Vec<DailyRow> {
    values
        .iter()
        .rev()
        .map(|(date, value)| DailyRow {
            date: date.clone(),
            date_label: parse_date(date).map(short_date).unwrap_or_else(|| date.clone()),
            value_label: label(*value),
        })
        .collect()
}

fn steps_panel(data: &HealthData, range: WeightRange, today: Date) -> DailyPanel {
    let series = daily_series(&data.steps, range, today).into_iter().map(|(date, steps)| (date, steps as f64)).collect::<Vec<_>>();
    let mut summary = Vec::new();
    if let Some(average) = week_average(&data.steps, today) {
        summary.push(format!("Promedio de la semana: {}.", steps_label(average.round() as i64)));
    }
    summary.push(format!("Meta orientativa: {}.", steps_label(STEPS_GOAL)));
    DailyPanel {
        today_label: data.steps.get(&today.to_string()).map(|steps| fmt(*steps as f64, 0)).unwrap_or_else(|| "—".to_string()),
        today_caption: "Hoy".to_string(),
        summary: summary.join(" "),
        ranges: range_choices(range),
        chart: daily_chart(
            &series,
            Some((STEPS_GOAL as f64, format!("Meta {}", fmt(STEPS_GOAL as f64, 0)))),
            |tick| fmt(tick, 0),
            |steps| steps_label(steps.round() as i64),
        ),
        empty_text: "Cargá los pasos de al menos dos días para ver tu evolución.".to_string(),
        entries: daily_rows(&data.steps, steps_label),
        heart: None,
    }
}

fn sleep_panel(data: &HealthData, range: WeightRange, today: Date) -> DailyPanel {
    let series = daily_series(&data.sleep, range, today).into_iter().map(|(date, minutes)| (date, minutes as f64 / 60.0)).collect::<Vec<_>>();
    let (low, high) = SLEEP_RANGE_MINUTES;
    let mut summary = Vec::new();
    if let Some(average) = week_average(&data.sleep, today) {
        let average = average.round() as i64;
        let note = if average < low { ", menos de lo recomendado" } else if average > high { ", más de lo recomendado" } else { "" };
        summary.push(format!("Promedio de la semana: {}{note}.", sleep_label(average)));
    }
    // «h» no queda sola en la línea siguiente.
    summary.push(format!("Recomendado: {} a {}\u{a0}h.", low / 60, high / 60));
    DailyPanel {
        today_label: data.sleep.get(&today.to_string()).map(|minutes| sleep_label(*minutes)).unwrap_or_else(|| "—".to_string()),
        today_caption: "Anoche".to_string(),
        summary: summary.join(" "),
        ranges: range_choices(range),
        chart: daily_chart(
            &series,
            Some((low as f64 / 60.0, format!("Mínimo recomendado {} h", low / 60))),
            |tick| format!("{} h", fmt(tick, 1)),
            |hours| sleep_label((hours * 60.0).round() as i64),
        ),
        empty_text: "Cargá el sueño de al menos dos noches para ver tu evolución.".to_string(),
        entries: daily_rows(&data.sleep, sleep_label),
        heart: heart_view(data, today),
    }
}

fn heart_view(data: &HealthData, today: Date) -> Option<HeartView> {
    let heart = sleep_heart(&data.sleep_bpm, today)?;
    let label = format!("Pulso dormido {} lpm", fmt(heart.bpm, 0));
    let Some(baseline) = heart.baseline else {
        return Some(HeartView { label, detail: "Con 5 noches más se compara con tu promedio.".to_string(), tone: "muted".to_string() });
    };
    let delta = heart.bpm - baseline;
    let detail = if heart.elevated() {
        format!("{} lpm más que tu promedio de 2 semanas ({}): puede ser cansancio o el comienzo de un resfrío; hoy conviene entrenar liviano.", fmt(delta, 0), fmt(baseline, 0))
    } else {
        format!("Tu promedio de 2 semanas es {} lpm.", fmt(baseline, 0))
    };
    Some(HeartView { label, detail, tone: if heart.elevated() { "warn" } else { "ok" }.to_string() })
}

/// Los pesos del último mes en una línea con medio kilo de aire arriba y abajo.
fn weight_trend(data: &HealthData, vitals: &Vitals, today: Date) -> WeightTrend {
    let from = days_ago(today, 30);
    let month = data
        .weights
        .iter()
        .filter_map(|entry| parse_date(&entry.date).map(|date| (date, entry.kg)))
        .filter(|(date, _)| *date >= from)
        .map(|(_, kg)| kg)
        .collect::<Vec<_>>();
    let goal_label = data
        .objective
        .target_kg
        .zip(vitals.current_kg)
        .filter(|(target, current)| (current - target).abs() >= 0.1)
        .map(|(target, current)| format!("Objetivo {} kg, faltan {}", fmt(target, 1), fmt((current - target).abs(), 1)));
    if month.len() < 2 {
        return WeightTrend { change_label: None, goal_label, line: None };
    }
    let (low, high) = month.iter().fold((f64::MAX, f64::MIN), |(low, high), kg| (low.min(*kg), high.max(*kg)));
    let (top, span) = (high + 0.5, high - low + 1.0);
    let last = (month.len() - 1) as f64;
    let points = month.iter().enumerate().map(|(index, kg)| (index as f64 / last * 100.0, (top - kg) / span * 100.0)).collect::<Vec<_>>();
    WeightTrend {
        change_label: Some(format!("{} kg en 30 días", signed(month[month.len() - 1] - month[0], 1))),
        goal_label,
        line: Some(monotone_path(&points)),
    }
}

fn svg_number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

/// Un trazo suave que no se pasa de los puntos (la curva «monótona en x» de
/// d3, la del canvas). Escalar los ejes no la cambia, así que se arma en la
/// caja de 100 × 100. `points` va de izquierda a derecha, con dos o más.
fn monotone_path(points: &[(f64, f64)]) -> String {
    let point = |(x, y): (f64, f64)| format!("{},{}", svg_number(x), svg_number(y));
    let mut path = format!("M{}", point(points[0]));
    if points.len() == 2 {
        return format!("{path}L{}", point(points[1]));
    }
    let slope = |index: usize| (points[index + 1].1 - points[index].1) / (points[index + 1].0 - points[index].0);
    let sign = |value: f64| if value < 0.0 { -1.0 } else { 1.0 };
    let last = points.len() - 1;
    let mut tangents = vec![0.0; points.len()];
    for index in 1..last {
        let (before, after) = (points[index].0 - points[index - 1].0, points[index + 1].0 - points[index].0);
        let (s0, s1) = (slope(index - 1), slope(index));
        let mean = (s0 * after + s1 * before) / (before + after);
        let tangent = (sign(s0) + sign(s1)) * s0.abs().min(s1.abs()).min(0.5 * mean.abs());
        tangents[index] = if tangent.is_nan() { 0.0 } else { tangent };
    }
    tangents[0] = (3.0 * slope(0) - tangents[1]) / 2.0;
    tangents[last] = (3.0 * slope(last - 1) - tangents[last - 1]) / 2.0;
    for index in 0..last {
        let ((x0, y0), (x1, y1)) = (points[index], points[index + 1]);
        let third = (x1 - x0) / 3.0;
        path.push_str(&format!(
            "C{} {} {}",
            point((x0 + third, y0 + third * tangents[index])),
            point((x1 - third, y1 - third * tangents[index + 1])),
            point((x1, y1))
        ));
    }
    path
}

fn plan_view(plan: &Plan) -> PlanView {
    PlanView {
        kcal_label: fmt(plan.kcal, 0),
        source_label: match plan.source {
            PlanSource::Ai => "Plan con IA",
            PlanSource::Calculated => "Plan calculado",
        }
        .to_string(),
        weeks_label: (plan.weeks > 0).then(|| format!("Tiempo estimado: {} semanas.", plan.weeks)),
        summary: plan.summary.clone(),
        notes: plan.notes.clone(),
        recommendations: plan.recommendations.clone(),
    }
}

fn objective_panel(data: &HealthData, vitals: &Vitals, today: Date) -> ObjectivePanel {
    let target = data.objective.target_kg;
    let target_bmi = target.zip(data.profile.as_ref()).and_then(|(target, profile)| super::bmi(target, profile.height_cm));
    ObjectivePanel {
        target_kg: target,
        pace: data.objective.pace,
        paces: PACES
            .iter()
            .map(|pace| Choice {
                value: pace.to_string(),
                label: format!("{} kg", fmt(*pace, 2)),
                selected: (pace - data.objective.pace).abs() < 1e-6,
            })
            .collect(),
        difference_label: target.zip(vitals.current_kg).map(|(target, current)| format!("{} kg", signed(target - current, 1))),
        target_bmi_label: target_bmi.map(|bmi| fmt(bmi, 1)),
        target_state: target_bmi.map(|bmi| {
            let (label, tone) = bmi_state(bmi);
            chip(label, tone)
        }),
        low_bmi_warning: target_bmi.is_some_and(|bmi| bmi < 18.5),
        can_plan: target.is_some() && vitals.tdee.is_some(),
        plan: data.plan.as_ref().map(plan_view),
        adjust: adjust_view(data, vitals, today),
    }
}

fn adjust_view(data: &HealthData, vitals: &Vitals, today: Date) -> Option<AdjustView> {
    let profile = data.profile.as_ref()?;
    let activity_suggestion = activity_suggestion(data, today).filter(|suggestion| (suggestion.factor - profile.activity).abs() > 1e-6).map(|suggestion| {
        let training = match suggestion.exercise_days_per_week {
            Some(per_week) => format!(" y {} entrenamientos por semana", fmt(per_week, 1)),
            None => " (sin entrenamientos en Gimnasio)".to_string(),
        };
        ActivitySuggestionView {
            value: suggestion.factor.to_string(),
            label: activity_label(suggestion.factor).unwrap_or_default().to_string(),
            reason: format!(
                "Promedio de {} pasos por día{training} en las últimas 4 semanas ({} días con pasos).",
                fmt(suggestion.steps_average as f64, 0),
                suggestion.step_days
            ),
        }
    });
    let (measured_label, measured_detail) = match &vitals.expenditure {
        ExpenditureStatus::Ready(measured) => (
            Some(format!("{} kcal", fmt(measured.kcal, 0))),
            format!(
                "Comiste {} kcal por día en promedio ({} días cargados) y tu peso {} {} kg por semana ({} pesajes) en las últimas 4 semanas. Solo vale si cargaste todas las comidas de esos días.",
                fmt(measured.intake, 0),
                measured.logged_days,
                if measured.kg_per_week < 0.0 { "bajó" } else { "subió" },
                fmt(measured.kg_per_week.abs(), 2),
                measured.weighings
            ),
        ),
        ExpenditureStatus::Missing(reason) => (None, reason.clone()),
    };
    Some(AdjustView {
        activity_label: activity_label(profile.activity).unwrap_or_default().to_string(),
        activity_suggestion,
        formula_label: vitals.tdee_formula.map(|kcal| format!("{} kcal", fmt(kcal, 0))),
        can_use_measured: measured_label.is_some(),
        measured_label,
        measured_detail,
        using_measured: vitals.tdee_measured,
    })
}

fn macro_indicator(key: MacroKey, value: f64, target: f64) -> MacroIndicator {
    let percent = if target > 0.0 { value / target * 100.0 } else { 0.0 };
    let rest = target - value;
    MacroIndicator {
        key: key.id().to_string(),
        label: key.label().to_string(),
        value_label: fmt(value, 0),
        target_label: format!("/ {} g", fmt(target, 0)),
        percent_label: format!("{} %", fmt(percent, 0)),
        progress: percent.min(100.0),
        status: if rest >= 0.5 {
            format!("Faltan {} g", fmt(rest, 0))
        } else if rest <= -0.5 {
            format!("{} g por encima", fmt(-rest, 0))
        } else {
            "Objetivo cumplido".to_string()
        },
    }
}

fn pills(nutrition: &super::Nutrition) -> Vec<MacroPill> {
    MacroKey::ALL
        .iter()
        .map(|key| MacroPill {
            key: key.id().to_string(),
            short: key.short().to_string(),
            label: key.label().to_string(),
            value_label: format!("{} g", fmt(nutrition.get(*key), 1)),
        })
        .collect()
}

fn calories_summary(sum: &Nutrition, plan: &Plan) -> CaloriesSummary {
    let over = sum.kcal > plan.kcal;
    CaloriesSummary {
        total_label: fmt(sum.kcal, 0),
        target_label: format!("de {} kcal", fmt(plan.kcal, 0)),
        progress: if plan.kcal > 0.0 { (sum.kcal / plan.kcal * 100.0).min(100.0) } else { 0.0 },
        over,
        remaining_label: if over {
            format!("{} kcal por encima del objetivo", fmt(sum.kcal - plan.kcal, 0))
        } else {
            format!("Quedan {} kcal", fmt(plan.kcal - sum.kcal, 0))
        },
        macros: MacroKey::ALL.iter().map(|key| macro_indicator(*key, sum.get(*key), key.of_plan(plan))).collect(),
    }
}

fn food_panel(data: &HealthData, vitals: &Vitals, date: Option<&str>, today: Date) -> Option<FoodPanel> {
    let plan = active_plan(data, vitals)?;
    let date = date.and_then(parse_date).filter(|date| *date <= today).unwrap_or(today);
    let key = date.to_string();
    let meals = meals_of(data, &key);
    let CaloriesSummary { total_label, target_label, progress, over, remaining_label, macros } = calories_summary(&total(&meals), plan);
    let footnote = format!(
        "{}{}",
        if vitals.bmr_from_scale {
            "Metabolismo basal tomado de tu balanza."
        } else {
            "Metabolismo basal estimado con la fórmula de Mifflin-St Jeor."
        },
        if data.plan.is_none() { " Sin plan activo: los objetivos son de mantenimiento." } else { "" }
    );
    Some(FoodPanel {
        date: key,
        day_label: day_label(date, today),
        previous_date: shift_days(date, -1).to_string(),
        next_date: (date < today).then(|| shift_days(date, 1).to_string()),
        total_label,
        target_label,
        progress,
        over,
        remaining_label,
        stats: vec![
            StatRow {
                label: if vitals.tdee_measured { "Gasto diario medido" } else { "Gasto diario estimado" }.into(),
                value: format!("{} kcal", fmt(vitals.tdee.unwrap_or_default(), 0)),
            },
            StatRow { label: "Objetivo del día".into(), value: format!("{} kcal", fmt(plan.kcal, 0)) },
            StatRow { label: "Metabolismo basal".into(), value: format!("{} kcal", fmt(vitals.bmr.unwrap_or_default(), 0)) },
        ],
        footnote,
        macros,
        groups: MealCategory::ALL
            .iter()
            .map(|category| {
                let items = meals.iter().filter(|meal| meal.category == *category).collect::<Vec<_>>();
                let subtotal = items.iter().map(|meal| meal.nutrition.kcal).sum::<f64>();
                MealGroup {
                    id: category.id().to_string(),
                    label: category.label().to_string(),
                    kcal_label: (!items.is_empty()).then(|| format!("{} kcal", fmt(subtotal, 0))),
                    meals: items
                        .iter()
                        .map(|meal| MealRow {
                            id: meal.id.clone(),
                            name: meal.name.clone(),
                            kcal_label: format!("{} kcal", fmt(meal.nutrition.kcal, 0)),
                            pills: pills(&meal.nutrition),
                            values: recent_meal(meal),
                        })
                        .collect(),
                }
            })
            .collect(),
    })
}

fn liters(ml: i64) -> String {
    format!("{} L", fmt(ml as f64 / 1000.0, 2))
}

fn water_panel(data: &HealthData, vitals: &Vitals, today: Date) -> Option<WaterPanel> {
    data.profile.as_ref()?;
    let goal = vitals.water_goal_ml.unwrap_or(2500).max(1);
    let ml = data.water.get(&today.to_string()).copied().unwrap_or_default();
    let ratio = (ml as f64 / goal as f64).min(1.0);
    Some(WaterPanel {
        ml,
        ratio,
        percent_label: format!("{}%", (ratio * 100.0).round()),
        liters_label: liters(ml),
        goal_label: format!("de {}", liters(goal)),
        basis_label: if data.profile.as_ref().is_some_and(|profile| profile.water_goal_ml.is_some()) {
            "Meta elegida en tu perfil".to_string()
        } else {
            "Según tu altura y peso".to_string()
        },
        aria_label: format!("{ml} de {goal} ml"),
        week: (0..7)
            .rev()
            .map(|back| {
                let date = days_ago(today, back);
                let ml = data.water.get(&date.to_string()).copied().unwrap_or_default();
                WaterDay {
                    date: date.to_string(),
                    day_label: weekday_narrow(date).to_string(),
                    fill: (ml as f64 / goal as f64 * 100.0).min(100.0),
                    met: ml >= goal,
                    title: liters(ml),
                }
            })
            .collect(),
    })
}

fn composition_panel(data: &HealthData, vitals: &Vitals, sex: Sex, selected: Option<&str>) -> Option<CompositionPanel> {
    let measurements = &data.measurements;
    let index = selected
        .and_then(|date| measurements.iter().position(|measurement| measurement.date == date))
        .or(measurements.len().checked_sub(1))?;
    let measurement = &measurements[index];
    let previous = index.checked_sub(1).map(|previous| &measurements[previous]);
    let date = parse_date(&measurement.date);

    let mut parts = [("fat", "Grasa", "grasaKg"), ("muscle", "Músculo", "musculoKg"), ("bone", "Huesos", "huesosKg")]
        .iter()
        .filter_map(|(key, label, metric)| measurement.value(metric).map(|kg| (*key, *label, kg)))
        .collect::<Vec<_>>();
    let known = parts.iter().map(|(_, _, kg)| kg).sum::<f64>();
    if measurement.weight - known > 0.2 {
        parts.push(("other", "Otros", super::round1(measurement.weight - known)));
    }
    let total = parts.iter().map(|(_, _, kg)| kg).sum::<f64>().max(f64::MIN_POSITIVE);

    let groups = MetricGroup::ALL
        .iter()
        .map(|group| MetricGroupView {
            id: group.id().to_string(),
            title: group.title().to_string(),
            metrics: METRICS
                .iter()
                .filter(|metric| metric.group == *group)
                .map(|metric| {
                    let value = measurement.value(metric.key);
                    let delta = value.zip(previous.and_then(|previous| previous.value(metric.key))).map(|(now, before)| now - before);
                    MetricRow {
                        key: metric.key.to_string(),
                        label: metric.label.to_string(),
                        value_label: value.map(|value| fmt(value, metric.decimals)).unwrap_or_else(|| "—".to_string()),
                        unit: value.filter(|_| !metric.unit.is_empty()).map(|_| metric.unit.to_string()),
                        delta_label: delta.filter(|delta| delta.abs() > 0.001).map(|delta| signed(delta, metric.decimals)),
                        bar: value.and_then(|value| range_for(metric.key, sex).map(|range| range_bar(&range, Some(value), None, false))),
                        state: value.and_then(|value| state(metric.key, value, sex, vitals.age)).map(|(label, tone)| chip(label, tone)),
                    }
                })
                .collect(),
        })
        .collect();

    Some(CompositionPanel {
        date: measurement.date.clone(),
        subtitle: format!(
            "Medición del {}{}",
            date.map(long_date).unwrap_or_else(|| measurement.date.clone()),
            if previous.is_some() { ", comparada con la anterior" } else { "" }
        ),
        dates: measurements
            .iter()
            .rev()
            .map(|item| Choice {
                value: item.date.clone(),
                label: parse_date(&item.date).map(short_date).unwrap_or_else(|| item.date.clone()),
                selected: item.date == measurement.date,
            })
            .collect(),
        strip_title: format!("Tu peso de {} kg se reparte así", fmt(measurement.weight, 1)),
        parts: parts
            .iter()
            .map(|(key, label, kg)| BodyPart {
                key: key.to_string(),
                label: label.to_string(),
                detail: format!("{} kg ({} %)", fmt(*kg, 1), fmt(kg / total * 100.0, 0)),
                share: kg / total * 100.0,
            })
            .collect(),
        groups,
    })
}

fn profile_form(data: &HealthData, vitals: &Vitals) -> ProfileForm {
    let profile = data.profile.as_ref();
    let activity = profile.map(|profile| profile.activity).unwrap_or(super::DEFAULT_ACTIVITY);
    ProfileForm {
        birth_date: profile.map(|profile| profile.birth_date.clone()).unwrap_or_default(),
        sex: profile.map(|profile| profile.sex.id()).unwrap_or("M").to_string(),
        height_cm: profile.map(|profile| profile.height_cm),
        activity,
        weight_kg: vitals.current_kg,
        water_goal_liters: profile.and_then(|profile| profile.water_goal_ml).map(|ml| ml as f64 / 1000.0),
        water_goal_auto_label: vitals.water_goal_auto_ml.map(liters),
        activities: ACTIVITIES
            .iter()
            .map(|(value, label)| Choice { value: value.to_string(), label: label.to_string(), selected: activity_label(activity) == Some(label) })
            .collect(),
    }
}

fn recent_meal(meal: &super::Meal) -> RecentMeal {
    RecentMeal {
        category: meal.category.id().to_string(),
        name: meal.name.clone(),
        kcal: meal.nutrition.kcal,
        protein_g: meal.nutrition.protein_g,
        carbs_g: meal.nutrition.carbs_g,
        fat_g: meal.nutrition.fat_g,
        fiber_g: meal.nutrition.fiber_g,
    }
}

fn meal_form(data: &HealthData, hour: u32) -> MealForm {
    let suggested = MealCategory::for_hour(hour);
    let mut meals = data.meals.iter().collect::<Vec<_>>();
    meals.sort_by(|a, b| b.date.cmp(&a.date).then(b.created_at_ms.cmp(&a.created_at_ms)));
    let mut seen = std::collections::HashSet::new();
    let mut per_category = std::collections::HashMap::<MealCategory, usize>::new();
    let mut recent = Vec::new();
    for meal in meals {
        let key = (meal.category, super::fold(&meal.name));
        let count = per_category.entry(meal.category).or_default();
        if *count >= 5 || !seen.insert(key) {
            continue;
        }
        *count += 1;
        recent.push(recent_meal(meal));
    }
    MealForm {
        suggested_category: suggested.id().to_string(),
        categories: MealCategory::ALL
            .iter()
            .map(|category| Choice { value: category.id().to_string(), label: category.label().to_string(), selected: *category == suggested })
            .collect(),
        recent,
    }
}

fn measurement_fields() -> Vec<MeasurementFieldGroup> {
    MetricGroup::ALL
        .iter()
        .map(|group| MeasurementFieldGroup {
            id: group.id().to_string(),
            title: group.title().to_string(),
            fields: METRICS
                .iter()
                .filter(|metric| metric.group == *group)
                .map(|metric| MeasurementField { key: metric.key.to_string(), label: metric.label.to_string(), unit: metric.unit.to_string() })
                .collect(),
        })
        .collect()
}
