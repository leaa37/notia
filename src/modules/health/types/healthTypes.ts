/** Contrato de Salud: los DTO que arma Rust (`health`). */

export type Tone = 'ok' | 'warn' | 'alto' | 'bajo'
export type WeightRange = '30' | '90' | '365' | 'all'

export interface HealthContext {
  libraryId: string
  actorLibraryUserId: string
}

export interface DashboardQuery {
  foodDate: string | null
  weightRange: WeightRange
  measurementDate: string | null
  stepsRange: WeightRange
  sleepRange: WeightRange
}

export interface StateChip {
  label: string
  tone: Tone
}

export interface Choice {
  value: string
  label: string
  selected: boolean
}

export interface RangeBar {
  segments: Array<{ label: string; tone: Tone; flex: number; active: boolean }>
  markers: Array<{ position: number; kind: 'value' | 'goal' }>
  ticks: Array<{ position: number; label: string }>
}

export interface BmiPanel {
  valueLabel: string
  state: StateChip
  message: string
  bar: RangeBar
  todayLabel: string
  goal: { label: string; state: StateChip | null } | null
  healthyRangeLabel: string
}

export interface ChartTick {
  at: number
  label: string
}

export interface WeightChart {
  points: Array<{ x: number; y: number; label: string; valueLabel: string }>
  yTicks: ChartTick[]
  xTicks: ChartTick[]
  goal: ChartTick | null
}

export interface WeightPanel {
  currentLabel: string
  summary: string
  ranges: Choice[]
  chart: WeightChart | null
  emptyText: string
  entries: Array<{ date: string; dateLabel: string; kgLabel: string }>
}

/** Un registro diario con histórico, como el peso: los pasos o el sueño. */
export interface DailyPanel {
  todayLabel: string
  todayCaption: string
  summary: string
  ranges: Choice[]
  chart: WeightChart | null
  emptyText: string
  entries: Array<{ date: string; dateLabel: string; valueLabel: string }>
  /** Pulso dormido de anoche (solo en el sueño, si llegó del reloj). */
  heart: { label: string; detail: string; tone: 'ok' | 'warn' | 'muted' } | null
}

/** Lo que las últimas semanas dicen para ajustar el plan (Rust lo calcula). */
export interface AdjustView {
  activityLabel: string
  activitySuggestion: { value: string; label: string; reason: string } | null
  formulaLabel: string | null
  measuredLabel: string | null
  measuredDetail: string
  usingMeasured: boolean
  canUseMeasured: boolean
}

export interface PlanView {
  kcalLabel: string
  sourceLabel: string
  weeksLabel: string | null
  summary: string
  notes: string[]
  recommendations: string[]
}

export interface ObjectivePanel {
  targetKg: number | null
  pace: number
  paces: Choice[]
  differenceLabel: string | null
  targetBmiLabel: string | null
  targetState: StateChip | null
  lowBmiWarning: boolean
  canPlan: boolean
  plan: PlanView | null
  adjust: AdjustView | null
}

export interface MacroPill {
  key: string
  short: string
  label: string
  valueLabel: string
}

export interface MacroIndicator {
  key: string
  label: string
  valueLabel: string
  targetLabel: string
  percentLabel: string
  progress: number
  status: string
}

export interface FoodPanel {
  date: string
  dayLabel: string
  previousDate: string
  nextDate: string | null
  totalLabel: string
  targetLabel: string
  progress: number
  over: boolean
  remainingLabel: string
  stats: Array<{ label: string; value: string }>
  footnote: string
  macros: MacroIndicator[]
  groups: Array<{
    id: string
    label: string
    kcalLabel: string | null
    meals: Array<{ id: string; name: string; kcalLabel: string; pills: MacroPill[]; values: RecentMeal }>
  }>
}

/** Las calorías de hoy (tarjeta del celular), aunque la alimentación muestre otro día. */
export interface CaloriesSummary {
  totalLabel: string
  targetLabel: string
  progress: number
  over: boolean
  remainingLabel: string
  macros: MacroIndicator[]
}

/** El peso de los últimos 30 días (tarjeta del celular). */
export interface WeightTrend {
  changeLabel: string | null
  goalLabel: string | null
  /** Trazo SVG en una caja de 100 × 100; sin línea con menos de dos pesos. */
  line: string | null
}

export interface WaterPanel {
  ml: number
  ratio: number
  percentLabel: string
  litersLabel: string
  goalLabel: string
  basisLabel: string
  ariaLabel: string
  week: Array<{ date: string; dayLabel: string; fill: number; met: boolean; title: string }>
}

export interface CompositionPanel {
  date: string
  subtitle: string
  dates: Choice[]
  stripTitle: string
  parts: Array<{ key: string; label: string; detail: string; share: number }>
  groups: Array<{
    id: string
    title: string
    metrics: Array<{
      key: string
      label: string
      valueLabel: string
      unit: string | null
      deltaLabel: string | null
      bar: RangeBar | null
      state: StateChip | null
    }>
  }>
}

export interface ProfileForm {
  birthDate: string
  sex: string
  heightCm: number | null
  activity: number
  weightKg: number | null
  activities: Choice[]
  /** Meta de agua elegida, en litros; `null` si se calcula. */
  waterGoalLiters: number | null
  /** La calculada con altura y peso («3,3 L»), si hay datos. */
  waterGoalAutoLabel: string | null
}

export interface RecentMeal {
  category: string
  name: string
  kcal: number
  proteinG: number
  carbsG: number
  fatG: number
  fiberG: number
}

export interface HealthDashboard {
  today: string
  hasProfile: boolean
  header: { chips: string[]; bmi: StateChip | null }
  bmi: BmiPanel | null
  weight: WeightPanel
  objective: ObjectivePanel
  food: FoodPanel | null
  todayCalories: CaloriesSummary | null
  weightTrend: WeightTrend
  water: WaterPanel | null
  steps: DailyPanel
  sleep: DailyPanel
  composition: CompositionPanel | null
  profileForm: ProfileForm
  mealForm: { suggestedCategory: string; categories: Choice[]; recent: RecentMeal[] }
  measurementFields: Array<{ id: string; title: string; fields: Array<{ key: string; label: string; unit: string }> }>
}

export interface ProfileInput {
  birthDate: string
  sex: string
  heightCm: number | null
  activity: number | null
  weightKg: number | null
  /** Vacía o 0: la meta de agua se calcula. */
  waterGoalLiters: number | null
}

export interface MeasurementInput {
  date: string
  weight: number | null
  values: Record<string, number>
}

export interface MealInput {
  date: string
  category: string
  name: string
  kcal: number | null
  proteinG: number | null
  carbsG: number | null
  fatG: number | null
  fiberG: number | null
}

/** Los cambios que la pantalla le pide a Rust (`HealthMutation`). */
export type HealthMutation =
  | { kind: 'saveProfile'; input: ProfileInput }
  | { kind: 'addWeight'; date: string; kg: number | null }
  | { kind: 'deleteWeight'; date: string }
  | { kind: 'saveMeasurement'; input: MeasurementInput }
  | { kind: 'deleteMeasurement'; date: string }
  | { kind: 'setObjective'; targetKg: number | null; pace: number | null }
  | { kind: 'calculatePlan' }
  | { kind: 'clearPlan' }
  | { kind: 'addWater'; date: string; deltaMl: number }
  | { kind: 'setWater'; date: string; ml: number }
  /** Como se escribieron: Rust los lee («8.432», «7:30»). */
  | { kind: 'setSteps'; date: string; steps: string }
  | { kind: 'deleteSteps'; date: string }
  | { kind: 'setSleep'; date: string; duration: string }
  | { kind: 'deleteSleep'; date: string }
  | { kind: 'saveMeal'; id: string | null; input: MealInput }
  | { kind: 'deleteMeal'; id: string }
  | { kind: 'setActivity'; activity: number }
  | { kind: 'setMeasuredExpenditure'; enabled: boolean }

export interface MealEstimate {
  kcal: number
  proteinG: number
  carbsG: number
  fatG: number
  fiberG: number
}

export interface FieldError {
  field: string
  message: string
}

export interface HealthError {
  code: 'validation' | 'not-found' | 'ai' | 'storage'
  message: string
  fields: FieldError[]
}
