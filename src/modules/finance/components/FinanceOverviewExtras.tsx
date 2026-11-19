import { Plus } from 'lucide-react'
import { formatAmount, formatMonthName, formatShortDate } from '../engines/financeFormat'
import type { FinanceOverview, LoanRow } from '../types/financeScreen'
import { useFinanceEntry } from './entry/financeEntryContext'

/*
 * What the month adds to «Resumen»: its balance, the budgets and the loans.
 * Rust derives every figure (`finance_overview`); a row opens its form.
 */

const LOAN_STATUS: Record<LoanRow['status'], string> = {
  paid: 'Pagada',
  pending: 'Por pagar',
  overdue: 'Atrasada',
  none: 'Sin cuota este mes',
}

function loanLine(row: LoanRow): string {
  const installment = row.number !== null ? `Cuota ${row.number}/${row.installmentCount}` : `${row.paidCount}/${row.installmentCount} pagadas`
  const next = row.nextDueDate ? ` · próxima ${formatShortDate(row.nextDueDate)}` : ' · terminado'
  return `${installment} · resta ${formatAmount(row.remaining, row.currency, 0)}${row.status === 'paid' ? '' : next}`
}

export function BalanceCard({ data, month, phone }: { data: FinanceOverview; month: string; phone: boolean }) {
  const { rows, otherIncomes } = data.balance
  const card = phone ? 'finance-phone-card' : 'finance-card finance-balance'
  return <section className={card} aria-labelledby="finance-balance-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-balance-title">Saldo de {formatMonthName(month)}</h2>
      <span className="finance-card__sub">lo que entró menos lo que salió y lo que fue al ahorro</span>
    </div>
    {rows.length === 0 && <p className="finance-empty">Todavía no hay ingresos ni gastos en {formatMonthName(month)}.</p>}
    {rows.map((row) => <div key={row.currency} className="finance-balance__rows">
      <div className="finance-split-row"><span>Entró{Number(row.otherIncome) > 0 && Number(row.salary) > 0 && <span className="finance-muted"> · sueldo {formatAmount(row.salary, row.currency, 0)} y otros {formatAmount(row.otherIncome, row.currency, 0)}</span>}</span><strong>{formatAmount(row.income, row.currency)}</strong></div>
      <div className="finance-split-row"><span>Salió en gastos</span><strong>− {formatAmount(row.expenses, row.currency)}</strong></div>
      {Number(row.savings) !== 0 && <div className="finance-split-row"><span>{Number(row.savings) > 0 ? 'Fue al ahorro' : 'Salió del ahorro'}</span><strong>{Number(row.savings) > 0 ? '− ' : '+ '}{formatAmount(row.savings.replace('-', ''), row.currency)}</strong></div>}
      <div className="finance-split-row finance-balance__left"><span>Quedó</span><strong className={Number(row.left) < 0 ? 'finance-alert-text' : 'finance-teal'}>{formatAmount(row.left, row.currency)}</strong></div>
    </div>)}
    {otherIncomes.length > 0 && <div className="finance-largest">
      <span className="finance-mono-label">Otros ingresos</span>
      {otherIncomes.map((row) => <div key={row.id} className="finance-split-row finance-split-row--padded">
        <span>{row.description}<span className="finance-muted"> · {formatShortDate(row.effectiveDate)}</span></span>
        <strong className="finance-medium">{formatAmount(row.amount, row.currency)}</strong>
      </div>)}
    </div>}
  </section>
}

export function BudgetsCard({ data, phone }: { data: FinanceOverview; phone: boolean }) {
  const entry = useFinanceEntry()
  const card = phone ? 'finance-phone-card' : 'finance-card finance-budgets'
  const over = data.budgets.filter((row) => row.status === 'over').length
  return <section className={card} aria-labelledby="finance-budgets-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-budgets-title">Presupuestos</h2>
      {over > 0 ? <span className="finance-alert-text finance-small">{over === 1 ? '1 pasado' : `${over} pasados`}</span>
        : entry && <button type="button" className="finance-link-button" onClick={() => entry.openEntry({ kind: 'budget' })}><Plus size={14} aria-hidden="true" /> Nuevo</button>}
    </div>
    {data.budgets.length === 0 && <p className="finance-empty">Sin presupuestos. Poné un tope por categoría y Notia te avisa al llegar al 80 % y al pasarte.</p>}
    {data.budgets.map((row) => <button key={row.id} type="button" className="finance-budget" onClick={() => entry?.openEntry({ kind: 'budget', id: row.id })}>
      <span className="finance-bar-row__head">
        <span>{row.categoryName}</span>
        <span className="finance-bar-row__value">{formatAmount(row.spent, row.currency, 0)}<span className="finance-muted"> de {formatAmount(row.budget, row.currency, 0)}</span></span>
      </span>
      <span className="finance-bar"><span className={`finance-bar__fill finance-budget__fill--${row.status}`} style={{ width: `${Math.min(row.percent, 100)}%` }} /></span>
      <span className={`finance-small ${row.status === 'over' ? 'finance-alert-text' : row.status === 'near' ? 'finance-warn-text' : 'finance-muted'}`}>
        {row.status === 'over' ? `Te pasaste ${formatAmount(row.remaining.replace('-', ''), row.currency, 0)} (${Math.round(row.percent)} %)` : `Quedan ${formatAmount(row.remaining, row.currency, 0)} (${Math.round(row.percent)} % usado)`}
      </span>
    </button>)}
  </section>
}

export function LoansCard({ data, month, phone }: { data: FinanceOverview; month: string; phone: boolean }) {
  const entry = useFinanceEntry()
  const { loans } = data
  const card = phone ? 'finance-phone-card' : 'finance-card finance-loans'
  const paid = loans.paid.map(([currency, amount]) => formatAmount(amount, currency)).join(' · ')
  return <section className={card} aria-labelledby="finance-loans-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-loans-title">Préstamos pagados</h2>
      <span className="finance-strong">{paid || '—'}<span className="finance-card__sub"> · {formatMonthName(month)}</span></span>
    </div>
    {loans.rows.length === 0 && <p className="finance-empty">No hay préstamos activos.{entry && <> <button type="button" className="finance-link-button" onClick={() => entry.openEntry({ kind: 'loan' })}>Cargar uno</button></>}</p>}
    <div className="finance-list">
      {loans.rows.map((row) => <div key={row.id} className="finance-loan">
        <button type="button" className="finance-loan__main" onClick={() => entry?.openEntry({ kind: 'loan', id: row.id })}>
          <span className="finance-service__name">{row.name}{row.lender && <span className="finance-muted"> · {row.lender}</span>}</span>
          <span className="finance-card__sub">{loanLine(row)}</span>
        </button>
        <span className="finance-loan__side">
          {row.expected && <strong>{formatAmount(row.paid ?? row.expected, row.currency, 0)}</strong>}
          <span className={`finance-pill finance-pill--loan-${row.status}`}>{LOAN_STATUS[row.status]}</span>
          {entry && (row.status === 'pending' || row.status === 'overdue') && <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry.openEntry({ kind: 'loan-payment', prefill: { loanId: row.id } })}>Pagar</button>}
        </span>
      </div>)}
    </div>
  </section>
}
