import type { Money } from '../../modules/finance/types/financeScreen'

/*
 * Home dashboard as `home_dashboard` returns it (`app/src/home.rs`). Rust
 * reads every module and writes the labels; a card whose source failed
 * carries `error` instead of `data`.
 */

export interface HomeCard<T> {
  data?: T
  error?: string
}

export type HomePriority = 'overdue' | 'urgent' | 'high' | 'medium' | 'low'

export interface HomeAgendaDay {
  date: string
  weekday: string
  day: number
  /** «sábado 26». */
  label: string
  isToday: boolean
  hasEvents: boolean
}

export interface HomeAgendaEvent {
  id: string
  date: string
  /** «Lun 28», or «Vie 2 oct» in another month. */
  dayLabel: string
  time: string
  /** «09:00 – 09:45». */
  range: string
  title: string
  priority: HomePriority
  priorityLabel: string
}

export interface HomeAgenda {
  days: HomeAgendaDay[]
  events: HomeAgendaEvent[]
}

export interface HomeNote {
  id: string
  text: string
  done: boolean
}

export interface HomeNotes {
  items: HomeNote[]
  pending: number
}

export type HomeTaskChip = HomePriority | 'blocked'

export interface HomeTask {
  /** Logical path: the ticket's identity for the Pomodoro. */
  filePath: string
  /** Path the explorer shows, to open the ticket. */
  path: string
  title: string
  meta: string
  chip: HomeTaskChip
  chipLabel: string
}

export interface HomeFocus {
  filePath: string
  title: string
  /** The timer already has it selected. */
  selected: boolean
}

export interface HomeTasks {
  completed: number
  sprint: number
  review: number
  blocked: number
  urgentCount: number
  urgent: HomeTask[]
  focus?: HomeFocus | null
}

export interface HomeFinance {
  /** `YYYY-MM`. */
  month: string
  /** «septiembre». */
  monthLabel: string
  expenses: Money[]
  expenseCount: number
  income: Money[]
  /** What is left of the month: income minus expenses minus savings. */
  left: Money[]
  /** What is still to pay this month and is known exactly (card installments). */
  pending: Money[]
  /** What is still to pay without an amount yet: «2 servicios». */
  pendingNote: string | null
  pendingCount: number
  saved: Money[]
}

export interface HomeRoutineDay {
  letter: string
  /** «Lunes 21: 5 de 6». */
  label: string
  done: number
  total: number
  pct: number | null
  isToday: boolean
  isFuture: boolean
}

export interface HomeHabit {
  id: string
  name: string
  category: string
  done: boolean
  streak: number
}

export interface HomeRoutineGroup {
  id: string
  name: string
  done: number
  total: number
  habits: HomeHabit[]
}

export interface HomeRoutine {
  /** `YYYY-MM-DD`, the day a habit is checked for. */
  today: string
  monthLabel: string
  monthPct: number | null
  weekPct: number | null
  bestStreak: number
  pendingToday: number
  week: HomeRoutineDay[]
  routines: HomeRoutineGroup[]
}

export type HomeRecentKind = 'chat' | 'note' | 'meeting'

export interface HomeRecentItem {
  kind: HomeRecentKind
  title: string
  meta: string
  /** «Hoy», «Ayer», «Esta semana» or «12 sep». */
  when: string
  path?: string
  agentFile?: string
}

export interface HomeFolder {
  name: string
  path: string
}

export interface HomeRecent {
  items: HomeRecentItem[]
  folders: HomeFolder[]
}

export interface HomeDashboard {
  /** «Sábado 26 de septiembre · gaia». */
  dateLabel: string
  /** «Sábado 26 de septiembre»: the phone layout leaves the library out. */
  todayLabel: string
  greeting: string
  summary: string
  agenda: HomeCard<HomeAgenda>
  tasks: HomeCard<HomeTasks>
  finance: HomeCard<HomeFinance>
  routine: HomeCard<HomeRoutine>
  notes: HomeCard<HomeNotes>
  recent: HomeRecent
}
