import { useState } from 'react'
import { Check, ChevronLeft, ChevronRight } from 'lucide-react'
import {
  formatAmount,
  formatMoney,
  formatMoneyList,
  formatMonthName,
  formatPercent,
  formatShortDate,
  plural,
} from '../../engines/financeFormat'
import { movementAmount, movementTone } from '../../engines/financeMovementText'
import type { FinanceOverview, ReviewCard, SalarySegmentKey, SalaryUse, ServiceStatus, StatementRow } from '../../types/financeScreen'
import { BalanceCard, BudgetsCard, LoansCard } from '../FinanceOverviewExtras'
import { useFinanceEntry } from '../entry/financeEntryContext'

/*
 * Resumen in the space of a phone: the canvas board «Teléfono · Resumen».
 * Every figure comes from Rust (`finance_overview`); this only lays it out.
 */

interface Props {
  data: FinanceOverview
  month: string
  onOpenChat: (prompt: string | null) => void
  onShowMovements: () => void
}

export function PhoneOverview({ data, month, onOpenChat, onShowMovements }: Props) {
  return <div className="finance-phone">
    {data.review.length > 0 && <PhoneReview cards={data.review} onOpenChat={onOpenChat} />}
    <PhoneSalaryUse salary={data.salary} month={month} previousMonth={data.categories.previousMonth} />
    <PhoneTiles data={data} />
    <BalanceCard data={data} month={month} phone />
    <BudgetsCard data={data} phone />
    <PhoneCategories data={data} month={month} onOpenChat={onOpenChat} />
    <PhoneCardsPaid data={data} month={month} />
    <LoansCard data={data} month={month} phone />
    <PhoneServices data={data} />
    <PhoneLatest data={data} month={month} onShowMovements={onShowMovements} />
  </div>
}

/** «Para revisar»: one card at a time; each answer goes to the chat composer. */
function PhoneReview({ cards, onOpenChat }: { cards: ReviewCard[]; onOpenChat: (prompt: string) => void }) {
  const entry = useFinanceEntry()
  const [index, setIndex] = useState(0)
  const card = cards[Math.min(index, cards.length - 1)]
  const step = (offset: number) => setIndex((current) => (current + offset + cards.length) % cards.length)
  return <section className="finance-phone-card finance-phone-review" aria-labelledby="finance-phone-review-title">
    <div className="finance-phone-row">
      <div className="finance-phone-review__title">
        <span className="finance-dot" aria-hidden="true" />
        <h2 id="finance-phone-review-title">Para revisar · {cards.length}</h2>
      </div>
      {cards.length > 1 && <div className="finance-phone-review__pager">
        <button type="button" className="finance-phone-plain-icon" aria-label="Anterior" onClick={() => step(-1)}><ChevronLeft size={16} /></button>
        <span aria-live="polite">{index + 1} de {cards.length}</span>
        <button type="button" className="finance-phone-plain-icon" aria-label="Siguiente" onClick={() => step(1)}><ChevronRight size={16} /></button>
      </div>}
    </div>
    <span className="finance-phone-review__kind">{card.label}</span>
    <p className="finance-phone-review__text">{card.parts.map((part, partIndex) => part.strong ? <strong key={partIndex}>{part.text}</strong> : <span key={partIndex}>{part.text}</span>)}</p>
    <div className="finance-phone-actions">
      {card.actions.map((action) => <button key={action.label} type="button" className={`finance-phone-button${action.primary ? ' finance-phone-button--accent' : ''}`} onClick={() => {
        if (entry && action.optionId) void entry.resolve(card.id, action.optionId)
        else onOpenChat(action.prompt)
      }}>{action.label}</button>)}
    </div>
    <span className="finance-phone-sub">Tocá la respuesta y queda resuelto; lo que no tiene respuesta fija va al chat.</span>
  </section>
}

const SEGMENT_LABELS: Record<SalarySegmentKey, string> = {
  cards: 'Tarjetas',
  savings: 'Ahorro',
  services: 'Servicios aparte',
  loans: 'Préstamos',
  unregistered: 'Sin registrar',
}

function PhoneSalaryUse({ salary, month, previousMonth }: { salary: SalaryUse | null; month: string; previousMonth: string }) {
  if (!salary) {
    const previous = formatMonthName(previousMonth)
    return <section className="finance-phone-card" aria-labelledby="finance-phone-salary-title">
      <h2 id="finance-phone-salary-title" className="finance-phone-h2--small">¿A dónde fue el sueldo de {previous}?</h2>
      <p className="finance-phone-sub">Todavía no hay un recibo de sueldo de {previous}. Cargalo con «+» → «Sueldo» o mandáselo al asistente.</p>
    </section>
  }
  const paid = salary.paymentDate ? `Neto cobrado el ${formatShortDate(salary.paymentDate)}` : `Neto de ${formatMonthName(salary.period)}`
  return <section className="finance-phone-card" aria-labelledby="finance-phone-salary-title">
    <div className="finance-phone-stack">
      <h2 id="finance-phone-salary-title" className="finance-phone-h2--small">¿A dónde fue el sueldo de {formatMonthName(salary.period)}?</h2>
      <span className="finance-phone-figure finance-phone-figure--xl">{formatMoney({ amount: salary.net, currency: salary.currency })}</span>
      <span className="finance-phone-sub">{paid} · paga lo que vence en {formatMonthName(month)}</span>
    </div>
    <div className="finance-stack" role="img" aria-label={salary.segments.map((segment) => `${SEGMENT_LABELS[segment.key]} ${formatPercent(segment.percent)}`).join(', ')}>
      {salary.segments.filter((segment) => Number(segment.amount) > 0).map((segment) => (
        segment.key === 'unregistered'
          ? <div key={segment.key} className="finance-stack__rest" />
          : <div key={segment.key} className={`finance-stack__part finance-swatch--${segment.key}`} style={{ width: `${segment.percent ?? 0}%` }} />
      ))}
    </div>
    {salary.exceeded && <p className="finance-warn-text finance-phone-sub">Lo registrado supera el sueldo de {formatMonthName(salary.period)}.</p>}
    <div className="finance-phone-legend">
      {salary.segments.map((segment) => <div key={segment.key} className="finance-phone-row">
        <span className="finance-phone-legend__label"><i className={`finance-swatch finance-swatch--${segment.key}`} aria-hidden="true" />{SEGMENT_LABELS[segment.key]}</span>
        {Number(segment.amount) > 0
          ? <span><strong>{formatAmount(segment.amount, salary.currency, 0)}</strong> <span className="finance-muted">{formatPercent(segment.percent)}</span></span>
          : <span className="finance-muted">—</span>}
      </div>)}
    </div>
  </section>
}

/** The four figures of the month. */
function PhoneTiles({ data }: { data: FinanceOverview }) {
  const { expenses, saved } = data
  const contribution = saved.contributions[0]
  const reserve = saved.reserves.find((item) => item.currency === contribution?.currency) ?? saved.reserves[0]
  const reserveChange = reserve ? saved.contributions.find((money) => money.currency === reserve.currency) : undefined
  const boughtCurrency = saved.bought[0]?.currency
  const rate = saved.rate && saved.cost[0] ? `a ${formatAmount(saved.rate, saved.cost[0].currency, 0)} por ${boughtCurrency === 'USD' ? 'dólar' : boughtCurrency}` : null
  return <div className="finance-phone-tiles">
    <section className="finance-phone-tile"><span>Gastos del mes</span><strong>{formatMoneyList(expenses.totals, 0)}</strong><small>{plural(expenses.count, 'gasto')}</small></section>
    <section className="finance-phone-tile"><span>Ahorrado</span><strong className={contribution ? 'finance-figure--teal' : undefined}>{contribution ? `+ ${formatMoney(contribution, 0)}` : '—'}</strong><small>{rate ?? (contribution ? 'aportes a las reservas' : 'sin aportes este mes')}</small></section>
    <section className="finance-phone-tile"><span>En tarjeta, a pagar</span><strong>{expenses.cardUnpaidCount === 0 ? 'Nada' : formatMoneyList(expenses.cardUnpaid, 0)}</strong><small>{expenses.cardUnpaidCount === 0 ? 'sin gastos esperando' : `${plural(expenses.cardUnpaidCount, 'gasto')} esperando`}</small></section>
    <section className="finance-phone-tile">
      <span>{reserve ? reserve.name : 'Ahorro'}</span>
      <strong>{reserve ? formatAmount(reserve.balance, reserve.currency, 0) : '—'}</strong>
      <small>{reserveChange ? `+ ${formatMoney(reserveChange, 0)} este mes` : reserve ? 'sin aportes este mes' : 'sin reservas'}</small>
    </section>
  </div>
}

function PhoneCategories({ data, month, onOpenChat }: { data: FinanceOverview; month: string; onOpenChat: (prompt: string) => void }) {
  const { categories } = data
  return <section className="finance-phone-card" aria-labelledby="finance-phone-categories-title">
    <div className="finance-phone-row finance-phone-row--baseline">
      <h2 id="finance-phone-categories-title">En qué se gastó</h2>
      {!categories.hasPrevious && categories.rows.length > 0 && <span className="finance-phone-sub">sin datos de {formatMonthName(categories.previousMonth)}</span>}
    </div>
    {categories.rows.length === 0 && <p className="finance-phone-sub">No hay gastos en {formatMonthName(month)}.</p>}
    {categories.rows.map((row) => <div key={`${row.filter}-${row.currency}`} className="finance-phone-bar-row">
      <div className="finance-phone-row">
        <span>{row.name} <span className="finance-muted">· {row.count}</span></span>
        <span className="finance-phone-strong">{formatAmount(row.amount, row.currency, 0)} <span className="finance-muted finance-phone-normal">{formatPercent(row.percent, 0)}</span></span>
      </div>
      <div className="finance-bar"><div className={`finance-bar__fill${row.uncategorized ? ' finance-bar__fill--hatch' : ''}`} style={{ width: `${row.percent ?? 0}%` }} /></div>
    </div>)}
    {categories.categorizePrompt && <button type="button" className="finance-phone-button finance-phone-button--accent" onClick={() => onOpenChat(categories.categorizePrompt!)}>Categorizar {plural(categories.uncategorizedCount, 'gasto')} en el chat</button>}
  </section>
}

function statementStatus(statement: StatementRow): { text: string; warn: boolean; check: boolean } {
  const difference = formatAmount(statement.difference, statement.currency, 0)
  if (statement.check === 'missing') return { text: `faltan ${difference} en líneas`, warn: true, check: false }
  if (statement.check === 'extra') return { text: `sobran ${difference} en líneas`, warn: true, check: false }
  if (statement.discarded.length > 0) return { text: `incluye ${plural(statement.discardedDescriptions.length, 'descartado')}`, warn: false, check: false }
  return { text: 'cuadra', warn: false, check: true }
}

function PhoneCardsPaid({ data, month }: { data: FinanceOverview; month: string }) {
  const entry = useFinanceEntry()
  const { cards } = data
  return <section className="finance-phone-card finance-phone-card--list" aria-labelledby="finance-phone-cards-title">
    <div className="finance-phone-row finance-phone-row--baseline">
      <h2 id="finance-phone-cards-title">Tarjetas pagadas</h2>
      <span className="finance-phone-strong">{formatMoneyList(cards.totals)}</span>
    </div>
    {cards.statements.length === 0 && <p className="finance-phone-sub">No vence ningún resumen de tarjeta en {formatMonthName(month)}.</p>}
    {cards.statements.map((statement) => {
      const status = statementStatus(statement)
      return <button key={statement.id} type="button" className="finance-phone-line finance-phone-line--button" aria-label={`Editar el resumen ${statement.name}`} onClick={() => entry?.openEntry({ kind: 'card-statement', id: statement.id })}>
        <span className="finance-phone-line__main">
          <span>{statement.name}</span>
          <span className={`finance-phone-sub${status.warn ? ' finance-warn-text' : ''}`}>{status.check && <Check size={14} aria-hidden="true" className="finance-teal" />}{status.text}</span>
        </span>
        <span className="finance-phone-line__end">
          <strong>{formatAmount(statement.amount, statement.currency, 0)}</strong>
          <span className="finance-phone-sub">vence {formatShortDate(statement.dueDate)}</span>
        </span>
      </button>
    })}
  </section>
}

const SERVICE_STATUS: Record<ServiceStatus, string> = { paid: 'Pagado', pending: 'Pendiente', 'not-applicable': 'No corresponde' }

function PhoneServices({ data }: { data: FinanceOverview }) {
  const entry = useFinanceEntry()
  const { services } = data
  return <section className="finance-phone-card finance-phone-card--tight" aria-labelledby="finance-phone-services-title">
    <div className="finance-phone-row finance-phone-row--baseline">
      <h2 id="finance-phone-services-title">Servicios</h2>
      {services.rows.length > 0 && <span className={services.pendingCount > 0 ? 'finance-warn-text finance-phone-small' : 'finance-phone-sub'}>{services.pendingCount > 0 ? `${services.pendingCount} sin pago` : 'Todos pagados'}</span>}
    </div>
    {services.rows.length === 0 && <p className="finance-phone-sub">No hay servicios activos. Agregalos en «Administrar» o pedíselo al asistente.</p>}
    {services.rows.map((row) => <div key={row.serviceId} className="finance-phone-service">
      <span className="finance-phone-line__main">
        <span className="finance-phone-medium">{row.name}</span>
        <span className="finance-phone-sub">{row.paid ? `pagado ${formatAmount(row.paid, row.currency, 0)}` : `esperado ${formatAmount(row.expected, row.currency, 0)}`}</span>
      </span>
      <span className="finance-service__side">
        <span className={`finance-pill finance-pill--${row.status}`}>{SERVICE_STATUS[row.status]}</span>
        {entry && row.status === 'pending' && <button type="button" className="finance-phone-button finance-phone-button--accent" onClick={() => entry.openEntry({ kind: 'service-payment', prefill: { serviceId: row.serviceId } })}>Pagar</button>}
      </span>
    </div>)}
  </section>
}

function PhoneLatest({ data, month, onShowMovements }: { data: FinanceOverview; month: string; onShowMovements: () => void }) {
  return <section className="finance-phone-card finance-phone-card--list" aria-labelledby="finance-phone-latest-title">
    <div className="finance-phone-row finance-phone-row--baseline">
      <h2 id="finance-phone-latest-title">Últimos movimientos</h2>
      {data.movementCount > 0 && <button type="button" className="finance-link-button" onClick={onShowMovements}>{data.movementCount === 1 ? 'Ver el movimiento' : `Ver los ${data.movementCount}`}</button>}
    </div>
    {data.latest.length === 0 && <p className="finance-phone-sub">No hay movimientos en {formatMonthName(month)}.</p>}
    {data.latest.map((row) => {
      const tone = movementTone(row)
      const where = row.exchange ? `${row.exchange.intoSavings ? 'a' : 'desde'} ${row.exchange.reserveName}` : row.flagged ? 'para revisar' : row.accountName
      return <div key={row.id} className="finance-phone-line finance-phone-line--tight">
        <span className="finance-phone-line__main">
          <span className={`finance-phone-ellipsis${tone === 'off' ? ' finance-tone--off' : ''}`}>{row.description}</span>
          <span className={row.flagged ? 'finance-phone-sub finance-warn-text' : 'finance-phone-sub'}>{formatShortDate(row.purchaseDate)} · {where}</span>
        </span>
        <strong className={`finance-phone-nowrap finance-tone--${tone}`}>{movementAmount(row, 0)}</strong>
      </div>
    })}
  </section>
}
