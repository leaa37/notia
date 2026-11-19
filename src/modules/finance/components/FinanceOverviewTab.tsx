import { useCallback, useState } from 'react'
import { Check } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceOverview } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import {
  formatAmount,
  formatMoney,
  formatMoneyList,
  formatMonthName,
  formatPercent,
  formatShortDate,
  formatShortMonth,
  formatSignedPercent,
  plural,
} from '../engines/financeFormat'
import { movementAmount, movementMeta, movementTone } from '../engines/financeMovementText'
import type { FinanceOverview, SalarySegmentKey, SalaryUse, ServiceStatus, StatementRow } from '../types/financeScreen'
import { FinanceReviewSection } from './FinanceReviewSection'
import { FinanceStatus } from './FinanceStatus'
import { BalanceCard, BudgetsCard, LoansCard } from './FinanceOverviewExtras'
import { useFinanceEntry } from './entry/financeEntryContext'
import { PhoneOverview } from './phone/PhoneOverview'

interface Props {
  library: NotiaLibrary
  month: string
  /** Today's official selling rate, shown beside what the dollars cost. */
  officialSell: number | null
  onOpenChat: (prompt: string | null) => void
  onShowMovements: () => void
  /** The phone board of the canvas. */
  phone: boolean
}

export function FinanceOverviewTab({ library, month, officialSell, onOpenChat, onShowMovements, phone }: Props) {
  const load = useCallback(() => getFinanceOverview(library, month), [library, month])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo cargar el resumen de Finanzas.')
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  if (phone) {
    return <>
      {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
      <PhoneOverview data={data} month={month} onOpenChat={onOpenChat} onShowMovements={onShowMovements} />
    </>
  }
  return <div className="finance-overview">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    {data.review.length > 0 && <FinanceReviewSection cards={data.review} onOpenChat={onOpenChat} />}
    <div className="finance-overview__grid">
      <SalaryUseCard salary={data.salary} month={month} />
      <div className="finance-overview__side">
        <ExpensesCard data={data} />
        <SavedCard data={data} officialSell={officialSell} />
      </div>
      <BalanceCard data={data} month={month} phone={false} />
      <BudgetsCard data={data} phone={false} />
      <CategoriesCard data={data} month={month} />
      <CardsPaidCard data={data} month={month} />
      <LoansCard data={data} month={month} phone={false} />
      <ServicesCard data={data} month={month} />
      <LatestCard data={data} month={month} onShowMovements={onShowMovements} />
    </div>
  </div>
}

// ---------------------------------------------------------------------------
// Salary

const SEGMENT_LABELS: Record<SalarySegmentKey, string> = {
  cards: 'Tarjetas pagadas',
  savings: 'Ahorro',
  services: 'Servicios aparte',
  loans: 'Préstamos',
  unregistered: 'Sin registrar',
}

function segmentCaption(key: SalarySegmentKey, amount: number, percent: number | null): string {
  if (key === 'services' && amount === 0) return 'Sin pagos fuera de tarjeta'
  if (key === 'unregistered') return `${formatPercent(percent)} · efectivo, transferencias o lo no cargado`
  return `${formatPercent(percent)} del sueldo`
}

function SalaryUseCard({ salary, month }: { salary: SalaryUse | null; month: string }) {
  const previous = formatMonthName(shiftPeriod(month, -1))
  if (!salary) {
    return <section className="finance-card finance-salary-use" aria-labelledby="finance-salary-use-title">
      <h2 id="finance-salary-use-title">¿A dónde fue el sueldo de {previous}?</h2>
      <p className="finance-empty">Todavía no hay un recibo de sueldo de {previous}. Cargalo con «Cargar» → «Sueldo» o mandáselo al asistente para ver a dónde fue.</p>
    </section>
  }
  const net = { amount: salary.net, currency: salary.currency }
  const bought = formatMoneyList(salary.savingsBought, 0)
  return <section className="finance-card finance-salary-use" aria-labelledby="finance-salary-use-title">
    <div className="finance-card__head">
      <div>
        <h2 id="finance-salary-use-title">¿A dónde fue el sueldo de {formatMonthName(salary.period)}?</h2>
        <p className="finance-card__sub">Con el sueldo cobrado {salary.paymentDate ? `el ${formatShortDate(salary.paymentDate)}` : `en ${formatMonthName(salary.period)}`} se paga lo que vence en {formatMonthName(month)}.</p>
      </div>
      <div className="finance-salary-use__total">
        <div className="finance-figure finance-figure--xl">{formatMoney(net)}</div>
        <div className="finance-card__sub">neto{salary.employers.length > 0 ? ` · ${salary.employers.join(' y ')}` : ''}</div>
      </div>
    </div>
    <div className="finance-stack" role="img" aria-label={salary.segments.map((segment) => `${SEGMENT_LABELS[segment.key]} ${formatPercent(segment.percent)}`).join(', ')}>
      {salary.segments.filter((segment) => Number(segment.amount) > 0).map((segment) => (
        segment.key === 'unregistered'
          ? <div key={segment.key} className="finance-stack__rest" />
          : <div key={segment.key} className={`finance-stack__part finance-swatch--${segment.key}`} style={{ width: `${segment.percent ?? 0}%` }} />
      ))}
    </div>
    {salary.exceeded && <p className="finance-warn-text">Lo registrado supera el sueldo de {formatMonthName(salary.period)}.</p>}
    <div className="finance-salary-use__legend">
      {salary.segments.map((segment) => {
        const amount = Number(segment.amount)
        const label = segment.key === 'savings' && bought !== '—' ? `Ahorro (${bought})` : SEGMENT_LABELS[segment.key]
        return <div key={segment.key} className={`finance-legend-item${segment.key === 'services' && amount === 0 ? ' finance-legend-item--empty' : ''}`}>
          <span className="finance-legend-item__label"><i className={`finance-swatch finance-swatch--${segment.key}`} aria-hidden="true" />{label}</span>
          <strong className="finance-legend-item__amount">{segment.key === 'services' && amount === 0 ? '—' : formatAmount(segment.amount, salary.currency)}</strong>
          <span className="finance-legend-item__caption">{segmentCaption(segment.key, amount, segment.percent)}</span>
        </div>
      })}
    </div>
  </section>
}

function shiftPeriod(period: string, offset: number): string {
  const [year, month] = period.split('-').map(Number)
  const date = new Date(year, month - 1 + offset, 1)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`
}

// ---------------------------------------------------------------------------
// Month figures

function ExpensesCard({ data }: { data: FinanceOverview }) {
  const { expenses } = data
  return <section className="finance-card finance-card--compact" aria-label="Gastos del mes">
    <span className="finance-card__sub">Gastos del mes</span>
    <span className="finance-figure finance-figure--lg">{formatMoneyList(expenses.totals)}</span>
    <span className="finance-card__sub">{plural(expenses.count, 'gasto')} · lo de tarjeta cuenta en el mes en que vence el resumen</span>
    <div className="finance-card__foot">
      {expenses.cardUnpaidCount === 0
        ? <span className="finance-check"><Check size={16} aria-hidden="true" />Nada en tarjeta esperando resumen</span>
        : <span className="finance-warn-text">{plural(expenses.cardUnpaidCount, 'gasto')} en tarjeta esperando su resumen · {formatMoneyList(expenses.cardUnpaid)}</span>}
    </div>
  </section>
}

function SavedCard({ data, officialSell }: { data: FinanceOverview; officialSell: number | null }) {
  const { saved } = data
  const contributions = saved.contributions.map((money) => `+ ${formatMoney(money)}`).join(' · ')
  const cost = formatMoneyList(saved.cost, 0)
  const boughtCurrency = saved.bought[0]?.currency
  return <section className="finance-card finance-card--compact" aria-label="Ahorrado este mes">
    <span className="finance-card__sub">Ahorrado este mes</span>
    <span className={`finance-figure finance-figure--lg${contributions ? ' finance-figure--teal' : ''}`}>{contributions || '—'}</span>
    <span className="finance-card__sub">
      {saved.cost.length > 0
        ? <>Costó {cost}{saved.rate && ` · ${formatAmount(saved.rate, saved.cost[0].currency, 0)} por ${boughtCurrency === 'USD' ? 'dólar' : boughtCurrency}`}{saved.rate && officialSell !== null && boughtCurrency === 'USD' && ` (oficial ${formatAmount(officialSell, 'ARS', 0)})`}</>
        : contributions ? 'Aportes a las reservas' : 'No hubo aportes este mes'}
      {saved.withdrawals.length > 0 && ` · Retirado: ${formatMoneyList(saved.withdrawals, 0)}`}
    </span>
    {saved.reserves.length > 0 && <div className="finance-card__foot finance-card__foot--list">
      {saved.reserves.map((reserve) => <div key={reserve.id} className="finance-split-row">
        <span>{reserve.name}</span>
        <strong>{formatAmount(reserve.balance, reserve.currency)}</strong>
      </div>)}
    </div>}
  </section>
}

// ---------------------------------------------------------------------------
// Categories

function CategoriesCard({ data, month }: { data: FinanceOverview; month: string }) {
  const { categories, largest } = data
  const previous = formatMonthName(categories.previousMonth)
  return <section className="finance-card finance-categories" aria-labelledby="finance-categories-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-categories-title">En qué se gastó</h2>
      {!categories.hasPrevious && categories.rows.length > 0 && <span className="finance-card__sub">{capitalize(previous)} sin datos para comparar</span>}
    </div>
    {categories.rows.length === 0 && <p className="finance-empty">No hay gastos en {formatMonthName(month)}.</p>}
    {categories.rows.map((row) => <div key={`${row.filter}-${row.currency}`} className="finance-bar-row">
      <div className="finance-bar-row__head">
        <span>{row.name}<span className="finance-muted"> · {plural(row.count, 'gasto')}{categories.hasPrevious && row.variation !== null && ` · ${formatSignedPercent(row.variation)} contra ${previous}`}</span></span>
        <span className="finance-bar-row__value">
          {formatAmount(row.amount, row.currency)}
          <span className="finance-muted"> {formatPercent(row.percent, 0)}</span>
        </span>
      </div>
      <div className="finance-bar"><div className={`finance-bar__fill${row.uncategorized ? ' finance-bar__fill--hatch' : ''}`} style={{ width: `${row.percent ?? 0}%` }} /></div>
      {row.description && <span className="finance-bar-row__note">{row.description}</span>}
    </div>)}
    {largest.length > 0 && <div className="finance-largest">
      <span className="finance-mono-label">Los más grandes</span>
      {largest.map((row) => <div key={row.id} className="finance-split-row finance-split-row--padded">
        <span>{row.description}<span className="finance-muted"> · {row.accountName}</span></span>
        <strong className="finance-medium">{formatAmount(row.amount, row.currency)}</strong>
      </div>)}
    </div>}
  </section>
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

// ---------------------------------------------------------------------------
// Cards paid

function statementStatus(statement: StatementRow): { text: string; warn: boolean; check: boolean } {
  const difference = formatAmount(statement.difference, statement.currency)
  if (statement.check === 'missing') return { text: `Faltan ${difference} en líneas cargadas`, warn: true, check: false }
  if (statement.check === 'extra') return { text: `Las líneas suman ${difference} más que el total`, warn: true, check: false }
  if (statement.discarded.length > 0) {
    return { text: `Incluye ${formatMoneyList(statement.discarded, 0)} descartado (${statement.discardedDescriptions.join(', ')})`, warn: false, check: false }
  }
  return { text: statement.lineCount === 1 ? 'Cuadra con su línea' : `Cuadra con sus ${statement.lineCount} líneas`, warn: false, check: true }
}

function CardsPaidCard({ data, month }: { data: FinanceOverview; month: string }) {
  const entry = useFinanceEntry()
  const { cards } = data
  const { installments } = cards
  return <section className="finance-card finance-cards-paid" aria-labelledby="finance-cards-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-cards-title">Tarjetas pagadas</h2>
      <span className="finance-strong">{formatMoneyList(cards.totals)}<span className="finance-card__sub"> · {plural(cards.statements.length, 'resumen', 'resúmenes')}</span></span>
    </div>
    {cards.statements.length === 0 && <p className="finance-empty">No vence ningún resumen de tarjeta en {formatMonthName(month)}.{entry && <> <button type="button" className="finance-link-button" onClick={() => entry.openEntry({ kind: 'card-statement' })}>Cargar un resumen</button></>}</p>}
    <div className="finance-list">
      {cards.statements.map((statement) => {
        const status = statementStatus(statement)
        return <button key={statement.id} type="button" className="finance-statement finance-statement--button" aria-label={`Editar el resumen ${statement.name}`} onClick={() => entry?.openEntry({ kind: 'card-statement', id: statement.id })}>
          <span className="finance-badge">{statement.badge}</span>
          <span className="finance-statement__main">
            <span className="finance-statement__name">{statement.name}</span>
            <span className={`finance-statement__status${status.warn ? ' finance-warn-text' : ''}`}>{status.check && <Check size={14} aria-hidden="true" className="finance-teal" />}{status.text}</span>
          </span>
          <span className="finance-statement__amount">
            <strong>{formatAmount(statement.amount, statement.currency)}</strong>
            <span className="finance-card__sub">vence {formatShortDate(statement.dueDate)}</span>
          </span>
        </button>
      })}
    </div>
    <p className="finance-card__sub">
      {installments.pendingCount === 0
        ? 'Cuotas: ninguna pendiente registrada.'
        : `Cuotas: ${installments.pendingCount} ${installments.pendingCount === 1 ? 'pendiente' : 'pendientes'} por ${formatMoneyList(installments.remaining, 0)}${installments.nextDueDate ? ` · la próxima vence el ${formatShortDate(installments.nextDueDate)}` : ''}.`}
    </p>
  </section>
}

// ---------------------------------------------------------------------------
// Services

const SERVICE_STATUS: Record<ServiceStatus, string> = { paid: 'Pagado', pending: 'Pendiente', 'not-applicable': 'No corresponde este mes' }
const HISTORY_STATUS: Record<ServiceStatus | 'missing', string> = { ...SERVICE_STATUS, 'not-applicable': 'No corresponde', missing: 'Sin registro' }

function ServicesCard({ data, month }: { data: FinanceOverview; month: string }) {
  const entry = useFinanceEntry()
  const { services } = data
  const [showHistory, setShowHistory] = useState(false)
  return <section className="finance-card finance-services" aria-labelledby="finance-services-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-services-title">Servicios de {formatMonthName(month)}</h2>
      {services.rows.length > 0 && <span className={services.pendingCount > 0 ? 'finance-warn-text finance-small' : 'finance-card__sub'}>{services.pendingCount > 0 ? `${services.pendingCount} sin pago` : 'Todos pagados'}</span>}
    </div>
    {services.rows.length === 0 && <p className="finance-empty">No hay servicios activos.{entry && <> <button type="button" className="finance-link-button" onClick={() => entry.openEntry({ kind: 'service' })}>Agregar uno</button></>}</p>}
    {services.rows.map((row) => <div key={row.serviceId} className="finance-service">
      <div className="finance-service__row">
        <span className="finance-service__main">
          <span className="finance-service__name">{row.name}</span>
          <span className="finance-card__sub">{[row.provider, row.paid ? `pagado ${formatAmount(row.paid, row.currency, 0)}` : `esperado ${formatAmount(row.expected, row.currency, 0)}`].filter(Boolean).join(' · ')}</span>
        </span>
        <span className="finance-service__side">
          <span className={`finance-pill finance-pill--${row.status}`}>{SERVICE_STATUS[row.status]}</span>
          {entry && row.status === 'pending' && <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry.openEntry({ kind: 'service-payment', prefill: { serviceId: row.serviceId } })}>Pagar</button>}
        </span>
      </div>
      {row.note && <p className="finance-service__note">{row.note}</p>}
    </div>)}
    {services.rows.length > 0 && <button type="button" className="finance-link-button" aria-expanded={showHistory} onClick={() => setShowHistory((value) => !value)}>{showHistory ? 'Ocultar historial de servicios' : 'Ver historial de servicios'}</button>}
    {showHistory && <div className="finance-table-scroll">
      <table className="finance-history">
        <caption className="finance-visually-hidden">Estado de cada servicio en los últimos meses</caption>
        <thead><tr><th scope="col">Servicio</th>{services.historyPeriods.map((period) => <th key={period} scope="col">{formatShortMonth(period)}</th>)}</tr></thead>
        <tbody>{services.rows.map((row) => <tr key={row.serviceId}>
          <th scope="row">{row.name}</th>
          {row.history.map((status, index) => <td key={services.historyPeriods[index]}><span className={`finance-history__dot finance-history__dot--${status}`} title={HISTORY_STATUS[status]} /><span className="finance-visually-hidden">{HISTORY_STATUS[status]}</span></td>)}
        </tr>)}</tbody>
      </table>
    </div>}
  </section>
}

// ---------------------------------------------------------------------------
// Latest movements

function LatestCard({ data, month, onShowMovements }: { data: FinanceOverview; month: string; onShowMovements: () => void }) {
  return <section className="finance-card finance-latest" aria-labelledby="finance-latest-title">
    <div className="finance-card__head finance-card__head--baseline">
      <h2 id="finance-latest-title">Últimos movimientos</h2>
      {data.movementCount > 0 && <button type="button" className="finance-link-button" onClick={onShowMovements}>Ver {data.movementCount === 1 ? 'el movimiento' : `los ${data.movementCount}`} →</button>}
    </div>
    {data.latest.length === 0 && <p className="finance-empty">No hay movimientos en {formatMonthName(month)}.</p>}
    <div className="finance-list">
      {data.latest.map((row) => {
        const tone = movementTone(row)
        return <div key={row.id} className="finance-movement">
          <span className="finance-movement__date">{formatShortDate(row.purchaseDate)}</span>
          <span className="finance-movement__main">
            <span className={`finance-movement__desc${tone === 'off' ? ' finance-tone--off' : ''}`}>{row.description}</span>
            <span className={row.flagged ? 'finance-movement__meta finance-warn-text' : 'finance-movement__meta'}>{movementMeta(row, true)}</span>
          </span>
          <span className="finance-movement__amount">
            <strong className={`finance-tone--${tone}`}>{movementAmount(row)}</strong>
            {row.exchange && <span className="finance-card__sub">{formatAmount(row.amount, row.currency)}</span>}
          </span>
        </div>
      })}
    </div>
  </section>
}
