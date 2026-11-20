import { useCallback } from 'react'
import { ArrowRight, TrendingDown, TrendingUp } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceOverview } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatExact, formatMoneyList, formatPercent, formatSignedPercent, plural } from '../engines/financeFormat'
import type { Balance, CategoryRow, Commitment, FinanceOverview, HistoryPoint } from '../types/financeScreen'
import { useFinanceEntry, type FinanceEntryApi } from './entry/financeEntryContext'
import { FinanceBar, FinanceCard, FinancePill, FinanceStatus, MoneyText, MovementList, PendingText } from './FinanceParts'
import type { FinanceTab } from './FinanceScreen'

interface Props {
  library: NotiaLibrary
  month: string
  onTab: (tab: FinanceTab) => void
}

/** «Resumen»: how the month goes, from what Rust adds up. */
export function FinanceOverviewTab({ library, month, onTab }: Props) {
  const load = useCallback(() => getFinanceOverview(library, month), [library, month])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo leer el resumen.')
  const entry = useFinanceEntry()
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  if (data.empty) return <Welcome entry={entry} />
  const pesos = data.balance.find((balance) => balance.currency === 'ARS') ?? null
  const others = data.balance.filter((balance) => balance.currency !== 'ARS')
  return <div className="finance-overview">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    <BalanceCard balance={pesos} others={others} />
    <div className="finance-grid">
      <CommitmentsCard view={data} entry={entry} onTab={onTab} />
      <PaceCard view={data} />
      <CategoriesCard rows={data.categories} />
      <HistoryCard points={data.history} current={data.month} />
      <DebtsCard view={data} onTab={onTab} />
      <SavingsCard view={data} onTab={onTab} />
      <FinanceCard title="Mayores gastos" className="finance-span-6">
        <MovementList rows={data.topExpenses} empty="Sin gastos este mes." />
      </FinanceCard>
      <FinanceCard title="Últimos movimientos" className="finance-span-6" action={<button type="button" className="finance-link-button" onClick={() => onTab('movements')}>Ver todos</button>}>
        <MovementList rows={data.recent} empty="Sin movimientos este mes." />
      </FinanceCard>
    </div>
  </div>
}

function Welcome({ entry }: { entry: FinanceEntryApi | null }) {
  const steps: Array<{ kind: string; title: string; text: string }> = [
    { kind: 'salary', title: 'Tu sueldo', text: 'Lo que cobrás en mano.' },
    { kind: 'expense', title: 'Un gasto', text: 'Nombre, importe y categoría.' },
    { kind: 'card', title: 'Tus tarjetas', text: 'Después les cargás las compras en cuotas.' },
    { kind: 'service', title: 'Tus servicios', text: 'Luz, internet, alquiler: los pagás cada mes.' },
  ]
  return <FinanceCard title="Empezá por acá" sub="Con esto, el Resumen te muestra cuánto te queda, qué falta pagar y en qué se va la plata.">
    <div className="finance-welcome">
      {steps.map((step) => <button key={step.kind} type="button" className="finance-welcome__step" onClick={() => entry?.openEntry({ kind: step.kind })}>
        <strong>{step.title}</strong>
        <span>{step.text}</span>
        <ArrowRight size={16} aria-hidden="true" />
      </button>)}
    </div>
  </FinanceCard>
}

function share(part: string, whole: string): number {
  const total = Number(whole)
  return total > 0 ? (Math.max(0, Number(part)) * 100) / total : 0
}

/** What came in and where it went, in pesos; other currencies apart. */
function BalanceCard({ balance, others }: { balance: Balance | null; others: Balance[] }) {
  if (!balance) {
    return <FinanceCard className="finance-balance"><p className="finance-empty">Sin movimientos en pesos este mes.</p>{others.map((other) => <OtherCurrency key={other.currency} balance={other} />)}</FinanceCard>
  }
  const income = Number(balance.income)
  const left = Number(balance.left)
  const change = balance.expensesChange
  return <section className="finance-card finance-balance" aria-label="Saldo del mes">
    <div className="finance-balance__top">
      <div className="finance-balance__main">
        <span className="finance-mono-label">{income > 0 ? (left >= 0 ? 'Te queda' : 'Gastaste de más') : 'Gastaste'}</span>
        <strong className={`finance-figure finance-figure--hero${left < 0 && income > 0 ? ' finance-figure--alert' : ''}`}>
          {formatAmount(income > 0 ? balance.left : balance.expenses, 'ARS', 0)}
        </strong>
        {income > 0 && <span className="finance-card__sub">de {formatAmount(balance.income, 'ARS', 0)} que entraron</span>}
      </div>
      <div className="finance-kpis">
        <Kpi label="Ingresos" value={formatAmount(balance.income, 'ARS', 0)} note={Number(balance.salary) > 0 ? `Sueldo ${formatAmount(balance.salary, 'ARS', 0)}` : 'Sin sueldo cargado'} />
        <Kpi
          label="Gastos"
          value={formatAmount(balance.expenses, 'ARS', 0)}
          note={change === null ? (balance.spentPercent === null ? '' : `${formatPercent(balance.spentPercent, 0)} del ingreso`) : `${formatSignedPercent(change, 0)} que el mes pasado`}
          tone={change === null ? undefined : change > 0 ? 'up' : 'down'}
        />
        <Kpi label="Ahorro" value={formatAmount(balance.saved, 'ARS', 0)} note={balance.savedPercent === null ? '' : `${formatPercent(balance.savedPercent, 0)} del ingreso`} />
      </div>
    </div>
    {income > 0 && <>
      <div className="finance-stack" role="img" aria-label={`Gastos ${formatPercent(share(balance.expenses, balance.income), 0)}, ahorro ${formatPercent(share(balance.saved, balance.income), 0)}`}>
        <span className="finance-stack__part finance-stack__part--spent" style={{ width: `${Math.min(100, share(balance.expenses, balance.income))}%` }} />
        <span className="finance-stack__part finance-stack__part--saved" style={{ width: `${Math.min(100, share(balance.saved, balance.income))}%` }} />
      </div>
      <div className="finance-legend">
        <span><i className="finance-swatch finance-swatch--spent" />Gastos {formatPercent(share(balance.expenses, balance.income), 0)}</span>
        <span><i className="finance-swatch finance-swatch--saved" />Ahorro {formatPercent(share(balance.saved, balance.income), 0)}</span>
        <span><i className="finance-swatch finance-swatch--left" />Queda {formatPercent(Math.max(0, share(balance.left, balance.income)), 0)}</span>
      </div>
    </>}
    {others.map((other) => <OtherCurrency key={other.currency} balance={other} />)}
  </section>
}

function OtherCurrency({ balance }: { balance: Balance }) {
  const money = (amount: string) => formatAmount(amount, balance.currency, 0)
  return <p className="finance-balance__other">
    <span className="finance-mono-label">En dólares</span>
    Entraron {money(balance.income)} · gastaste {money(balance.expenses)} · ahorraste {money(balance.saved)} · queda {money(balance.left)}
  </p>
}

function Kpi({ label, value, note, tone }: { label: string; value: string; note: string; tone?: 'up' | 'down' }) {
  return <div className="finance-kpi">
    <span className="finance-kpi__label">{label}</span>
    <strong className="finance-kpi__value">{value}</strong>
    {note && <span className={`finance-kpi__note${tone ? ` finance-kpi__note--${tone}` : ''}`}>
      {tone === 'up' && <TrendingUp size={13} aria-hidden="true" />}
      {tone === 'down' && <TrendingDown size={13} aria-hidden="true" />}
      {note}
    </span>}
  </div>
}

const STATUS_TEXT: Record<Commitment['status'], string> = { paid: 'Pagado', pending: 'Pendiente', late: 'Atrasado' }

function CommitmentsCard({ view, entry, onTab }: { view: FinanceOverview; entry: FinanceEntryApi | null; onTab: (tab: FinanceTab) => void }) {
  const { commitments } = view
  const total = commitments.paidCount + commitments.pendingCount
  const pay = (item: Commitment) => {
    if (item.kind === 'service') entry?.openEntry({ kind: 'service-payment', prefill: { serviceId: item.id, month: view.month } })
    else if (item.kind === 'card') entry?.openEntry({ kind: 'card-payment', prefill: { cardId: item.id, month: view.month } })
    else entry?.openEntry({ kind: 'loan-payment', prefill: { loanId: item.id } })
  }
  return <FinanceCard
    className="finance-span-7"
    title="Pagos del mes"
    sub={total === 0 ? 'Sin servicios, tarjetas ni préstamos este mes.' : `Pagaste ${commitments.paidCount} de ${total}`}
    action={<button type="button" className="finance-link-button" onClick={() => onTab('payments')}>Ir a Pagos</button>}
  >
    {total > 0 && <>
      <FinanceBar percent={(commitments.paidCount * 100) / total} label={`Pagaste ${commitments.paidCount} de ${total}`} />
      <div className="finance-split">
        <div><span className="finance-mono-label">Pagado</span><strong><MoneyText list={commitments.paid} empty="$ 0" /></strong></div>
        <div><span className="finance-mono-label">Falta pagar</span><strong className={commitments.pendingCount > 0 ? 'finance-warn-text' : ''}><PendingText pending={commitments.pending} note={commitments.pendingNote} /></strong></div>
      </div>
      <ul className="finance-commitments" role="list">
        {commitments.items.map((item) => <li key={`${item.kind}:${item.id}:${item.currency}`} className="finance-commitment">
          <span className="finance-commitment__main">
            <span className="finance-commitment__title">{item.title}</span>
            <span className="finance-commitment__sub">{item.kind === 'service' ? 'Servicio' : item.kind === 'card' ? 'Tarjeta' : 'Préstamo'} · {item.subtitle}</span>
          </span>
          <span className="finance-commitment__amount">{item.amount ? formatExact(item.amount, item.currency) : ''}</span>
          {item.status === 'paid'
            ? <FinancePill tone="paid">{STATUS_TEXT.paid}</FinancePill>
            : <button type="button" className={`finance-button finance-button--small ${item.status === 'late' ? 'finance-button--warn' : 'finance-button--outline'}`} onClick={() => pay(item)}>
              {item.status === 'late' ? 'Atrasado · Pagar' : 'Pagar'}
            </button>}
        </li>)}
      </ul>
    </>}
  </FinanceCard>
}

function PaceCard({ view }: { view: FinanceOverview }) {
  const pace = view.pace
  return <FinanceCard className="finance-span-5" title="Ritmo del mes" sub={pace ? `Día ${pace.daysElapsed} de ${pace.daysInMonth}` : undefined}>
    {!pace ? <p className="finance-empty">Sin gastos en pesos este mes.</p> : <>
      <FinanceBar percent={(pace.daysElapsed * 100) / pace.daysInMonth} tone="muted" label={`Día ${pace.daysElapsed} de ${pace.daysInMonth}`} />
      <dl className="finance-facts">
        <div><dt>Gasto por día</dt><dd>{formatAmount(pace.dailyAverage, 'ARS', 0)}</dd></div>
        {pace.projected && <div><dt>Si seguís así, el mes cierra en</dt><dd>{formatAmount(pace.projected, 'ARS', 0)}</dd></div>}
        <div><dt>Gastos cargados</dt><dd>{pace.expenseCount}</dd></div>
        {pace.biggest && <div><dt>El más grande</dt><dd>{pace.biggest.name} · {formatAmount(pace.biggest.amount, 'ARS', 0)}</dd></div>}
      </dl>
    </>}
  </FinanceCard>
}

function budgetTone(row: CategoryRow): 'teal' | 'warn' | 'alert' {
  if (!row.budget) return 'teal'
  return row.budget.status === 'over' ? 'alert' : row.budget.status === 'near' ? 'warn' : 'teal'
}

function CategoriesCard({ rows }: { rows: CategoryRow[] }) {
  const entry = useFinanceEntry()
  return <FinanceCard
    className="finance-span-7"
    title="En qué se fue"
    sub="Por categoría, con su presupuesto si tiene."
    action={<button type="button" className="finance-link-button" onClick={() => entry?.openEntry({ kind: 'budget' })}>Nuevo presupuesto</button>}
  >
    {rows.length === 0 ? <p className="finance-empty">Sin gastos este mes.</p> : <ul className="finance-categories" role="list">
      {rows.map((row) => {
        const budget = row.budget
        return <li key={`${row.currency}:${row.categoryId ?? 'none'}`} className="finance-category">
          <div className="finance-category__head">
            <span className="finance-category__name">{row.name}<span className="finance-muted"> · {plural(row.count, 'gasto')}</span></span>
            <span className="finance-category__amount">{formatAmount(row.amount, row.currency, 0)}<span className="finance-muted"> {formatPercent(row.percent, 0)}</span></span>
          </div>
          <FinanceBar percent={budget ? budget.percent : row.percent} tone={budgetTone(row)} label={budget ? `${formatPercent(budget.percent, 0)} del presupuesto` : `${formatPercent(row.percent, 0)} de los gastos`} />
          {budget && <span className={`finance-category__budget finance-category__budget--${budget.status}`}>
            {budget.status === 'over'
              ? `Te pasaste ${formatAmount(String(-Number(budget.remaining)), budget.currency, 0)} del presupuesto de ${formatAmount(budget.budget, budget.currency, 0)}`
              : `Quedan ${formatAmount(budget.remaining, budget.currency, 0)} de ${formatAmount(budget.budget, budget.currency, 0)}`}
          </span>}
        </li>
      })}
    </ul>}
  </FinanceCard>
}

/** Six months of income and expenses, in pesos. */
function HistoryCard({ points, current }: { points: HistoryPoint[]; current: string }) {
  const max = Math.max(1, ...points.flatMap((point) => [Number(point.income), Number(point.expenses)]))
  const average = points.filter((point) => Number(point.expenses) > 0)
  const mean = average.length > 0 ? average.reduce((sum, point) => sum + Number(point.expenses), 0) / average.length : 0
  return <FinanceCard className="finance-span-5" title="Últimos 6 meses" sub={mean > 0 ? `Gastás ${formatAmount(String(mean), 'ARS', 0)} por mes en promedio` : 'En pesos'}>
    <div className="finance-chart" role="img" aria-label={points.map((point) => `${point.label}: entró ${formatAmount(point.income, 'ARS', 0)}, gastaste ${formatAmount(point.expenses, 'ARS', 0)}`).join('; ')}>
      {points.map((point) => <div key={point.month} className={`finance-chart__month${point.month === current ? ' finance-chart__month--current' : ''}`}>
        <div className="finance-chart__bars">
          <span className="finance-chart__bar finance-chart__bar--income" style={{ height: `${(Number(point.income) * 100) / max}%` }} title={`Ingresos ${formatAmount(point.income, 'ARS', 0)}`} />
          <span className="finance-chart__bar finance-chart__bar--expenses" style={{ height: `${(Number(point.expenses) * 100) / max}%` }} title={`Gastos ${formatAmount(point.expenses, 'ARS', 0)}`} />
        </div>
        <span className="finance-chart__label">{point.label}</span>
      </div>)}
    </div>
    <div className="finance-legend">
      <span><i className="finance-swatch finance-swatch--income" />Ingresos</span>
      <span><i className="finance-swatch finance-swatch--spent" />Gastos</span>
    </div>
  </FinanceCard>
}

function DebtsCard({ view, onTab }: { view: FinanceOverview; onTab: (tab: FinanceTab) => void }) {
  const { debts } = view
  const nothing = debts.cards.length === 0 && debts.loans.length === 0
  return <FinanceCard className="finance-span-6" title="Lo que debés" sub={nothing ? undefined : debts.cardTotal.length > 0 ? `En tarjetas: ${formatMoneyList(debts.cardTotal, 0)}` : undefined} action={<button type="button" className="finance-link-button" onClick={() => onTab('payments')}>Ver Pagos</button>}>
    {nothing ? <p className="finance-empty">Sin cuotas pendientes.</p> : <>
      {debts.cards.map((card) => <div key={card.id} className="finance-debt">
        <span className="finance-debt__name">{card.name}</span>
        <span className="finance-debt__detail">{plural(card.installmentsLeft, 'cuota')} · hasta {card.lastMonth}</span>
        <strong><MoneyText list={card.remaining} /></strong>
      </div>)}
      {debts.upcoming.some((due) => due.amounts.length > 0) && <div className="finance-upcoming">
        <span className="finance-mono-label">Cuotas de tarjeta que vienen</span>
        {debts.upcoming.map((due) => <div key={due.month} className="finance-upcoming__row"><span>{due.label}</span><strong><MoneyText list={due.amounts} empty="—" /></strong></div>)}
      </div>}
      {debts.loans.map((loan) => <div key={loan.id} className="finance-loan">
        <div className="finance-loan__head"><span>{loan.name}</span><span className="finance-muted">{loan.paidCount}/{loan.count} cuotas · pagaste {formatAmount(loan.totalPaid, loan.currency, 0)}</span></div>
        <FinanceBar percent={loan.percent} label={`${loan.paidCount} de ${loan.count} cuotas`} />
      </div>)}
    </>}
  </FinanceCard>
}

function SavingsCard({ view, onTab }: { view: FinanceOverview; onTab: (tab: FinanceTab) => void }) {
  return <FinanceCard className="finance-span-6" title="Ahorro" action={<button type="button" className="finance-link-button" onClick={() => onTab('savings')}>Ver Ahorro</button>}>
    {view.savings.length === 0 ? <p className="finance-empty">Sin reservas de ahorro.</p> : <div className="finance-split">
      {view.savings.map((total) => <div key={total.currency}>
        <span className="finance-mono-label">{total.currency === 'USD' ? 'En dólares' : 'En pesos'}</span>
        <strong className="finance-figure">{formatAmount(total.balance, total.currency, 0)}</strong>
        <span className={`finance-card__sub${Number(total.savedThisMonth) > 0 ? ' finance-teal' : ''}`}>
          {Number(total.savedThisMonth) === 0 ? 'Nada en el mes' : `${Number(total.savedThisMonth) > 0 ? '+' : ''}${formatAmount(total.savedThisMonth, total.currency, 0)} en el mes`}
        </span>
      </div>)}
    </div>}
  </FinanceCard>
}
