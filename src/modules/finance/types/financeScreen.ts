// What the Finanzas screen shows, derived by Rust (`finance_screen.rs`).
// Amounts are decimal strings with two decimals; each currency goes apart.

export interface Money {
  currency: string
  amount: string
}

export interface ReviewTextPart {
  text: string
  strong: boolean
}

export interface ReviewAction {
  label: string
  /** Message for the chat composer; nothing changes until it is sent and confirmed. */
  prompt: string
  primary: boolean
  /** The option Rust applies at once; `null` when it is answered in the chat. */
  optionId: string | null
}

export interface ReviewCard {
  id: string
  label: string
  parts: ReviewTextPart[]
  actions: ReviewAction[]
}

export interface ExchangeDetail {
  amount: string
  currency: string
  reserveName: string
  rate: string | null
  intoSavings: boolean
}

export interface MovementRow {
  id: string
  kind: 'expense' | 'income' | 'exchange' | 'transfer' | 'adjustment' | string
  status: string
  description: string
  purchaseDate: string
  effectiveDate: string
  countsOnStatementDue: boolean
  accountId: string
  accountName: string
  categoryName: string | null
  uncategorized: boolean
  serviceName: string | null
  origin: string
  viaTelegram: boolean
  amount: string
  currency: string
  exchange: ExchangeDetail | null
  flagged: boolean
  note: string | null
}

export type SalarySegmentKey = 'cards' | 'savings' | 'services' | 'loans' | 'unregistered'

export interface SalaryUse {
  period: string
  paymentDate: string | null
  employers: string[]
  currency: string
  net: string
  segments: Array<{ key: SalarySegmentKey; amount: string; percent: number | null }>
  savingsBought: Money[]
  exceeded: boolean
}

export interface CategoryRow {
  filter: string
  name: string
  description: string | null
  currency: string
  amount: string
  count: number
  percent: number | null
  variation: number | null
  uncategorized: boolean
}

export interface StatementRow {
  id: string
  accountId: string
  badge: string
  name: string
  amount: string
  currency: string
  dueDate: string
  lineCount: number
  check: 'matches' | 'missing' | 'extra'
  difference: string
  discarded: Money[]
  discardedDescriptions: string[]
}

export type ServiceStatus = 'paid' | 'pending' | 'not-applicable'

export interface ServiceRow {
  serviceId: string
  name: string
  provider: string | null
  currency: string
  expected: string
  paid: string | null
  status: ServiceStatus
  note: string | null
  history: Array<ServiceStatus | 'missing'>
}

export interface BalanceRow {
  currency: string
  income: string
  salary: string
  otherIncome: string
  expenses: string
  savings: string
  left: string
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

export interface LoanRow {
  id: string
  name: string
  lender: string | null
  currency: string
  number: number | null
  expected: string | null
  paid: string | null
  status: 'paid' | 'pending' | 'overdue' | 'none'
  paidCount: number
  installmentCount: number
  remaining: string
  nextDueDate: string | null
}

export interface FinanceOverview {
  month: string
  review: ReviewCard[]
  salary: SalaryUse | null
  balance: { rows: BalanceRow[]; otherIncomes: MovementRow[] }
  budgets: BudgetRow[]
  /** `paid`: what was paid in the month, as `[currency, amount]`. */
  loans: { rows: LoanRow[]; paid: Array<[string, string]>; pendingCount: number }
  expenses: { totals: Money[]; count: number; cardUnpaid: Money[]; cardUnpaidCount: number }
  saved: {
    contributions: Money[]
    bought: Money[]
    cost: Money[]
    withdrawals: Money[]
    rate: string | null
    reserves: Array<{ id: string; name: string; currency: string; balance: string }>
  }
  categories: {
    rows: CategoryRow[]
    previousMonth: string
    hasPrevious: boolean
    uncategorizedCount: number
    categorizePrompt: string | null
  }
  largest: MovementRow[]
  cards: {
    totals: Money[]
    statements: StatementRow[]
    installments: { pendingCount: number; remaining: Money[]; nextDueDate: string | null }
  }
  services: { pendingCount: number; rows: ServiceRow[]; historyPeriods: string[] }
  latest: MovementRow[]
  movementCount: number
}

export interface FilterChip {
  id: string
  label: string
  count: number
}

export interface MovementGroup {
  accountId: string
  badge: string
  name: string
  statement: { total: Money; dueDate: string } | null
  kindLabel: string
  totals: Money[]
  savings: Money[]
  rows: MovementRow[]
}

export interface FinanceMovements {
  month: string
  filter: string
  chips: FilterChip[]
  summary: { expenseCount: number; expenseTotals: Money[]; exchangeCount: number; incomeCount: number }
  groups: MovementGroup[]
  changePrompts: Record<string, string>
}

export type ProductSort = 'recent' | 'rising' | 'az'

export interface ProductRow {
  id: string
  name: string
  merchantCount: number
  lastDate: string
  fromPrice: Money
  changePercent: number | null
  firstDate: string
}

export interface MerchantPriceRow {
  merchant: string
  price: string
  date: string
  cheapest: boolean
  difference: string | null
  differencePercent: number | null
}

export interface ProductDetail {
  id: string
  name: string
  aliases: string[]
  currency: string
  best: MerchantPriceRow
  worst: MerchantPriceRow | null
  changePercent: number | null
  changeMerchant: string | null
  series: Array<{ merchant: string; points: Array<{ date: string; price: string }> }>
  rows: MerchantPriceRow[]
  similar: ReviewCard | null
  correctPrompt: string
}

export type TicketStatus = 'card-unpaid' | 'paid-in-statement' | 'confirmed' | 'pending' | 'discarded'

export interface TicketRow {
  id: string
  date: string
  merchant: string
  itemCount: number
  payment: string
  status: TicketStatus
  total: Money
}

export interface FinanceProducts {
  totalCount: number
  products: ProductRow[]
  selected: ProductDetail | null
  tickets: TicketRow[]
}

export type SalaryShareKey = 'cards' | 'services' | 'savings'

export interface SalaryShare {
  key: SalaryShareKey
  current: number | null
  amount: Money | null
  salary: Money | null
  periods: string[]
  values: Array<number | null>
}

export interface QuoteCard {
  kind: string
  name: string
  buy: number
  sell: number
  spread: number
  updatedAt: string
  gapPercent: number | null
}

export interface ReserveView {
  id: string
  name: string
  currency: string
  balance: string
  monthChange: string
  valuation: Array<{ kind: string; amount: string }>
  totals: Array<{ kind: string; amount: string }>
  movements: Array<{
    id: string
    date: string
    reason: string
    movementType: string
    amount: string
    currency: string
    cost: Money | null
    rate: string | null
  }>
}

export interface FinanceSalarySavings {
  month: string
  salary: {
    employers: string[]
    bars: Array<{ period: string; ars: number; usd: number | null }>
    averageArs: number | null
    averageUsd: number | null
    latest: {
      period: string
      net: string
      currency: string
      paymentDate: string | null
      previousPeriod: string | null
      changePercent: number | null
    } | null
    dollarError: string | null
  }
  inflation: { from: string; to: string; arsChange: number; usdChange: number; ipc: number; arsVsIpc: number; usdVsIpc: number } | null
  inflationError: string | null
  shares: SalaryShare[]
  quotes: QuoteCard[]
  quotesError: string | null
  lastPurchase: { date: string; rate: string; vsOfficial: number | null; vsBlue: number | null } | null
  reserves: ReserveView[]
}
