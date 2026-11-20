// What the tabs of Finanzas show, as `finance_screen.rs` builds it. Every
// figure comes from Rust; the screen only draws it.

export interface Money {
  currency: string
  amount: string
}

export interface MonthNav {
  month: string
  label: string
  previous: string
  next: string
  isCurrent: boolean
}

export interface MovementRow {
  id: string
  kind: 'expense' | 'income'
  name: string
  categoryId: string | null
  category: string | null
  account: string | null
  amount: string
  currency: string
  date: string
  /** What it pays: «Cuota 3/12 · Visa», «Servicio de noviembre 2026». */
  detail: string | null
  tag: 'card' | 'loan' | 'service' | 'salary' | null
}

export interface Balance {
  currency: string
  income: string
  salary: string
  otherIncome: string
  expenses: string
  saved: string
  left: string
  spentPercent: number | null
  savedPercent: number | null
  previousExpenses: string
  expensesChange: number | null
}

export interface Pace {
  daysElapsed: number
  daysInMonth: number
  dailyAverage: string
  projected: string | null
  expenseCount: number
  biggest: MovementRow | null
}

export type CommitmentStatus = 'paid' | 'pending' | 'late'

export interface Commitment {
  kind: 'service' | 'card' | 'loan'
  id: string
  title: string
  subtitle: string
  amount: string | null
  currency: string
  status: CommitmentStatus
}

export interface Commitments {
  items: Commitment[]
  paid: Money[]
  /** What is still to pay and is known exactly (card installments). */
  pending: Money[]
  /** What is still to pay without an amount yet: «2 servicios». */
  pendingNote: string | null
  paidCount: number
  pendingCount: number
}

export interface BudgetRow {
  id: string
  categoryId: string
  categoryName: string
  currency: string
  budget: string
  spent: string
  remaining: string
  percent: number
  status: 'ok' | 'near' | 'over'
}

export interface CategoryRow {
  categoryId: string | null
  name: string
  currency: string
  amount: string
  percent: number
  count: number
  budget: BudgetRow | null
}

export interface HistoryPoint {
  month: string
  label: string
  income: string
  expenses: string
  saved: string
}

export interface CardDebt {
  id: string
  name: string
  remaining: Money[]
  installmentsLeft: number
  lastMonth: string | null
}

export interface MonthDue {
  month: string
  label: string
  amounts: Money[]
}

export interface LoanProgress {
  id: string
  name: string
  currency: string
  amount: string
  paidCount: number
  count: number
  totalPaid: string
  percent: number
}

export interface SavingsTotal {
  currency: string
  balance: string
  savedThisMonth: string
}

export interface FinanceOverview extends MonthNav {
  balance: Balance[]
  pace: Pace | null
  commitments: Commitments
  categories: CategoryRow[]
  history: HistoryPoint[]
  debts: { cards: CardDebt[]; cardTotal: Money[]; upcoming: MonthDue[]; loans: LoanProgress[] }
  savings: SavingsTotal[]
  topExpenses: MovementRow[]
  recent: MovementRow[]
  empty: boolean
}

export interface Payment {
  movementId: string
  date: string
  amount: string
}

/** A service is only its name: the amount is the one of the month's payment. */
export interface ServicePayment {
  id: string
  name: string
  paid: Payment | null
  currency: string | null
}

export interface InstallmentRow {
  purchaseId: string
  name: string
  number: number
  count: number
  month: string
  amount: string
  currency: string
  paid: Payment | null
  late: boolean
}

export interface PurchaseRow {
  id: string
  name: string
  category: string | null
  total: string
  currency: string
  count: number
  paidCount: number
  remaining: string
  firstMonth: string
  lastMonth: string
  finished: boolean
}

export interface CardMonth {
  id: string
  name: string
  installments: InstallmentRow[]
  total: Money[]
  pending: Money[]
  pendingCount: number
  debt: Money[]
  purchases: PurchaseRow[]
}

export interface LoanMonth {
  id: string
  name: string
  currency: string
  amount: string
  count: number
  number: number | null
  paid: Payment | null
  nextNumber: number | null
  paidCount: number
  totalPaid: string
  lateCount: number
  finished: boolean
}

export interface FinancePayments extends MonthNav {
  services: ServicePayment[]
  cards: CardMonth[]
  loans: LoanMonth[]
  paid: Money[]
  /** What is still to pay and is known exactly (card installments). */
  pending: Money[]
  /** What is still to pay without an amount yet. */
  pendingNote: string | null
}

export interface MovementFilter {
  kind: '' | 'expense' | 'income'
  categoryId: string
  accountId: string
  search: string
}

export interface FilterOption {
  id: string
  name: string
}

export interface FinanceMovements extends MonthNav {
  rows: MovementRow[]
  income: Money[]
  expenses: Money[]
  categories: FilterOption[]
  accounts: FilterOption[]
}

export interface SearchMonth {
  month: string
  label: string
  rows: MovementRow[]
}

export interface FinanceSearch {
  count: number
  page: number
  hasMore: boolean
  months: SearchMonth[]
  income: Money[]
  expenses: Money[]
}

export interface ReserveRow {
  id: string
  name: string
  currency: string
  balance: string
  goal: string | null
  goalPercent: number | null
  savedThisMonth: string
}

export interface SavingsRow {
  id: string
  kind: 'savings' | 'exchange'
  title: string
  reserve: string
  direction: 'deposit' | 'withdrawal'
  amount: string
  currency: string
  pesos: string | null
  date: string
  note: string
}

export interface FinanceSavings {
  reserves: ReserveRow[]
  totals: SavingsTotal[]
  dollarsInPesos: { quote: string; rate: number; pesos: string } | null
  movements: SavingsRow[]
  quotes: Array<{ kind: string; name: string; buy: number; sell: number; updatedAt: string }>
  quotesError: string | null
}
