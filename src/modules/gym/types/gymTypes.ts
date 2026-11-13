/* Contratos de Gimnasio con Rust (backend-core::gym y app/src/gym.rs). */

export interface GymContext {
  libraryId: string
  actorLibraryUserId: string
}

/** El contexto de un cambio de la pantalla, con la marca de la vista que lo pide. */
export interface GymApplyContext extends GymContext {
  origin: string
}

/** El aviso de Rust: la biblioteca y, si el cambio lo pidió una vista, su marca. */
export interface GymChangedEvent {
  libraryId: string
  origin: string | null
}

export type GymScreen = 'panel' | 'rutinas' | 'editar' | 'entrenar' | 'equipo'

export interface GymQuery {
  screen: GymScreen
  routineId: string | null
  day: string | null
  search: string
  group: string | null
  onlyMine: boolean
  /** Al abrir: Rust arma Entrenar si hay un entrenamiento en curso. */
  resume?: boolean
}

export interface Choice {
  key: string
  label: string
}

export interface RoutineRow {
  id: string
  name: string
  focus: string
  count: string
  days: string
  color: string
  selected: boolean
}

export interface SessionBadge {
  routineId: string
  routineName: string
  status: 'running' | 'paused'
}

export interface StatCard {
  key: 'streak' | 'week' | 'kcal' | 'time'
  value: string
  sub: string
}

export type MuscleState = 'fat' | 'rec' | 'ok' | 'none'

export interface MuscleRow {
  key: string
  name: string
  state: MuscleState
  detail: string
}

export interface LegendRow {
  state: MuscleState
  label: string
  count: number
}

export interface HeatDay {
  date: string
  color: string | null
  level: 0 | 1 | 2 | 3
  future: boolean
  today: boolean
  selected: boolean
  label: string
}

export interface HeatWeek {
  month: string
  days: HeatDay[]
}

export interface DayDetail {
  date: string
  has: boolean
  routine: string
  color: string | null
  kcal: string
  minutes: string
  sets: string
  noneText: string
}

export interface Bar {
  letter: string
  day: number
  value: string
  height: number
  color: string | null
  today: boolean
  label: string
}

export interface PanelView {
  weekLabel: string
  nextLabel: string
  nextRoutineId: string | null
  nextName: string
  /** La próxima es la rutina de hoy y hoy todavía no entrenó. */
  nextIsToday: boolean
  stats: StatCard[]
  muscleStates: Record<string, MuscleState>
  muscles: MuscleRow[]
  legend: LegendRow[]
  advice: string
  weeks: HeatWeek[]
  day: DayDetail
  routineLegend: { name: string; color: string }[]
  bars: Bar[]
  calories14: string
}

export interface SetView {
  weight: string
  reps: string
  weightLabel: string
  repsLabel: string
  volumeLabel: string
}

export interface ItemView {
  id: string
  n: number
  exerciseId: string
  name: string
  group: string
  groupLabel: string
  equipmentLabel: string
  restS: number
  restLabel: string
  missing: string | null
  summary: string
  weighted: boolean
  timed: boolean
  sets: SetView[]
  totalLabel: string
}

export interface RoutineView {
  id: string
  name: string
  focus: string
  focusLabel: string
  color: string
  days: boolean[]
  items: ItemView[]
  stats: { exercises: number; sets: number; volume: string }
  summary: { muscles: Record<string, 1 | 2>; primary: string; secondary: string; rest: string }
  trainLabel: string
}

export interface LibraryRow {
  id: string
  name: string
  group: string
  meta: string
  missing: string | null
  added: boolean
}

export interface LibraryView {
  rows: LibraryRow[]
  hidden: number
  hiddenNote: string
}

export interface TrainSet {
  weight: string
  reps: string
  done: boolean
  next: boolean
}

export interface TrainItem {
  id: string
  n: number
  exerciseId: string
  name: string
  group: string
  groupLabel: string
  restLabel: string
  missing: string | null
  weighted: boolean
  timed: boolean
  count: string
  allDone: boolean
  current: boolean
  sets: TrainSet[]
}

export interface TrainingView {
  routineId: string
  name: string
  focusLabel: string
  status: 'idle' | 'running' | 'paused' | 'done'
  startedMs: number
  accMs: number
  restEndMs: number | null
  restLeftMs: number | null
  restTotalMs: number
  doneSets: number
  totalSets: number
  percent: number
  volume: string
  kcalPerMin: number
  items: TrainItem[]
  nextText: string | null
  currentRest: string | null
  doneText: string
}

export interface EquipmentRow {
  id: string
  name: string
  category: string
  categoryLabel: string
  sub: string
  owned: boolean
  custom: boolean
  hasImage: boolean
}

export interface EquipmentView {
  items: EquipmentRow[]
  presets: { id: string; label: string; pressed: boolean }[]
  available: string
  availablePercent: number
  routines: { name: string; color: string; ok: boolean; status: string; issues: { exercise: string; need: string }[] }[]
  categories: Choice[]
}

export interface GymView {
  /** La pantalla que Rust armó: la pedida, o Entrenar al retomar. */
  screen: GymScreen
  sex: 'masculino' | 'femenino'
  sexFromProfile: boolean
  routines: RoutineRow[]
  routineId: string | null
  equipmentOwned: number
  equipmentTotal: number
  exerciseTotal: number
  session: SessionBadge | null
  groups: Choice[]
  panel: PanelView | null
  routine: RoutineView | null
  library: LibraryView | null
  training: TrainingView | null
  equipment: EquipmentView | null
  nowMs: number
}

export interface GymApplyResult {
  view: GymView
  routineId: string | null
}

export interface ExerciseDetail {
  id: string
  name: string
  group: string
  groupLabel: string
  tracking: string
  weighted: boolean
  timed: boolean
  kcal: string
  image: string | null
  hasVideo: boolean
  steps: string[]
  muscles: Record<string, 1 | 2>
  muscleChips: { key: string; label: string; level: 0 | 1 | 2 }[]
  primaryText: string
  secondaryText: string
  meta: string
  equipment: { id: string; name: string; required: boolean }[]
  requiredText: string
  statusText: string
  missing: boolean
  projections: { label: string; value: string }[]
  path: string
}

export interface MusclePath {
  muscle: string
  d: string
}

export interface BodyFigure {
  viewBox: string
  silhouette: string[]
  outline: string[]
  muscles: MusclePath[]
}

export interface BodyView {
  sex: string
  front: BodyFigure | null
  back: BodyFigure | null
}

export type SetField = 'weight' | 'reps'

export type GymMutation =
  | { type: 'create-routine' }
  | { type: 'duplicate-routine'; routineId: string }
  | { type: 'delete-routine'; routineId: string }
  | { type: 'rename-routine'; routineId: string; name: string }
  | { type: 'set-focus'; routineId: string; focus: string }
  | { type: 'toggle-day'; routineId: string; day: number }
  | { type: 'add-exercise'; routineId: string; exerciseId: string }
  | { type: 'remove-item'; routineId: string; itemId: string }
  | { type: 'add-set'; routineId: string; itemId: string }
  | { type: 'remove-set'; routineId: string; itemId: string; index: number }
  | { type: 'set-value'; routineId: string; itemId: string; index: number; field: SetField; value: string }
  | { type: 'adjust-rest'; routineId: string; itemId: string; deltaS: number }
  | { type: 'toggle-equipment'; equipmentId: string }
  | { type: 'apply-preset'; preset: string }
  | { type: 'start-session'; routineId: string }
  | { type: 'pause-session' }
  | { type: 'resume-session' }
  | { type: 'finish-session' }
  | { type: 'restart-session' }
  | { type: 'toggle-set-done'; routineId: string; itemId: string; index: number }
  | { type: 'adjust-rest-timer'; deltaMs: number }
  | { type: 'skip-rest' }

export type ExerciseEdit =
  | { field: 'name'; value: string }
  | { field: 'group'; value: string }
  | { field: 'weighted'; value: boolean }
  | { field: 'timed'; value: boolean }
  | { field: 'cycle-muscle'; muscle: string }
  | { field: 'toggle-equipment'; equipmentId: string }
  | { field: 'kcal'; value: string }
  | { field: 'kcal-step'; delta: number }
  | { field: 'step'; index: number; text: string }
  | { field: 'add-step' }
  | { field: 'remove-step'; index: number }
  | { field: 'remove-media' }

export type CatalogMutation =
  | { type: 'create-exercise'; group: string | null }
  | { type: 'update-exercise'; exerciseId: string; edit: ExerciseEdit }
  | { type: 'save-equipment'; equipmentId: string | null; name: string; category: string }
  | { type: 'delete-equipment'; equipmentId: string }

export interface MediaInput {
  mediaType: string
  base64: string
}

export interface CatalogResult {
  exercise: ExerciseDetail | null
  equipmentId: string | null
}

export interface GymError {
  code: 'validation' | 'not-found' | 'storage'
  message: string
}
