import { useCallback, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceSalarySavings } from '../services/financeService'
import { notifyFinanceDataChanged } from '../services/financeDataEvents'
import { useFinanceResource } from '../hooks/useFinanceResource'
import {
  formatAmount,
  formatMillions,
  formatMoney,
  formatMonthName,
  formatPercent,
  formatShortDate,
  formatShortMonth,
  formatShortMonthYear,
  formatSignedPercent,
  formatUpdatedAt,
} from '../engines/financeFormat'
import {
  capitalize,
  joinSpanish,
  lastPurchaseComparison,
  missingReserveMovements,
  RESERVE_MOVEMENT_TYPES as MOVEMENT_TYPES,
  reserveMovementAdds,
} from '../engines/financeSalaryText'
import type { FinanceSalarySavings, QuoteCard, ReserveView, SalaryShare, SalaryShareKey } from '../types/financeScreen'
import { FinanceStatus } from './FinanceStatus'
import { PhoneSalary } from './phone/PhoneSalary'
import { useFinanceEntry } from './entry/financeEntryContext'

const SHORT_RANGE = 6
const BAR_MAX_HEIGHT = 200

/** `phone`: the phone board of the canvas. */
export function FinanceSalaryTab({ library, month, phone }: { library: NotiaLibrary; month: string; phone: boolean }) {
  const load = useCallback(() => getFinanceSalarySavings(library, month), [library, month])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudieron cargar el sueldo y el ahorro.')
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  if (phone) {
    return <>
      {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
      <PhoneSalary data={data} />
    </>
  }
  return <div className="finance-salary">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    <div className="finance-salary__grid">
      <SalaryChartCard data={data} />
      <div className="finance-salary__side">
        <AverageCard data={data} />
        <InflationCard data={data} />
      </div>
      <div className="finance-section-title">
        <h2>Qué parte del sueldo se va en…</h2>
        <span className="finance-card__sub">Cada mes se compara con el sueldo del mes anterior, que es con el que se paga.</span>
      </div>
      {data.shares.map((share) => <ShareCard key={share.key} share={share} />)}
      <QuotesCard data={data} />
      {data.reserves.map((reserve) => <ReserveCard key={reserve.id} reserve={reserve} />)}
      {data.reserves.length === 0 && <section className="finance-card finance-span-12"><h2>Ahorro</h2><p className="finance-empty">Todavía no hay reservas de ahorro. Creá una en «Administrar» o pedíselo al asistente.</p></section>}
    </div>
  </div>
}

// ---------------------------------------------------------------------------
// Salary

function SalaryChartCard({ data }: { data: FinanceSalarySavings }) {
  const { salary } = data
  const [inDollars, setInDollars] = useState(false)
  const [fullYear, setFullYear] = useState(false)
  const bars = salary.bars.slice(fullYear ? 0 : -SHORT_RANGE)
  const dollarsAvailable = salary.bars.length > 0 && salary.bars.every((bar) => bar.usd !== null)
  const useDollars = inDollars && dollarsAvailable
  const value = (bar: { ars: number; usd: number | null }) => (useDollars ? bar.usd ?? 0 : bar.ars)
  const average = useDollars ? salary.averageUsd : salary.averageArs
  const maximum = Math.max(1, ...bars.map(value), average ?? 0)
  const label = (amount: number) => (useDollars ? formatAmount(amount, 'USD', 0) : formatMillions(amount))
  const latest = salary.latest
  const range = bars.length > 0 ? `${formatMonthName(bars[0].period)} a ${formatMonthName(bars[bars.length - 1].period)} ${bars[bars.length - 1].period.slice(0, 4)}` : ''
  return <section className="finance-card finance-salary-chart" aria-labelledby="finance-salary-chart-title">
    <div className="finance-card__head">
      <div>
        <h2 id="finance-salary-chart-title">Sueldo neto</h2>
        <p className="finance-card__sub">{[salary.employers.join(' y '), range].filter(Boolean).join(' · ')}</p>
      </div>
      <div className="finance-segmented" role="group" aria-label="Cómo ver el sueldo">
        <button type="button" aria-pressed={!useDollars} onClick={() => setInDollars(false)}>Pesos</button>
        <button type="button" aria-pressed={useDollars} disabled={!dollarsAvailable} onClick={() => setInDollars(true)}>Dólares</button>
        <button type="button" aria-pressed={fullYear} onClick={() => setFullYear((current) => !current)}>12 meses</button>
      </div>
    </div>
    {bars.length === 0
      ? <p className="finance-empty">Todavía no hay recibos de sueldo. Mandale uno al asistente.</p>
      : <>
        <div className="finance-bars" role="img" aria-label={bars.map((bar) => `${formatShortMonthYear(bar.period)}: ${label(value(bar))}`).join(', ')}>
          {average !== null && <>
            <div className="finance-bars__average" style={{ bottom: `${(average / maximum) * BAR_MAX_HEIGHT}px` }} />
            <span className="finance-bars__average-label" style={{ bottom: `${(average / maximum) * BAR_MAX_HEIGHT + 6}px` }}>promedio {salary.bars.length} {salary.bars.length === 1 ? 'mes' : 'meses'} · {label(average)}</span>
          </>}
          {bars.map((bar, index) => <div key={bar.period} className={`finance-bars__bar${index === bars.length - 1 ? ' is-current' : ''}`}>
            <span>{label(value(bar))}</span>
            <div style={{ height: `${Math.max(2, (value(bar) / maximum) * BAR_MAX_HEIGHT)}px` }} />
          </div>)}
        </div>
        <div className="finance-bars__axis" aria-hidden="true">
          {bars.map((bar, index) => <span key={bar.period} className={index === bars.length - 1 ? 'is-current' : undefined}>{formatShortMonth(bar.period)}</span>)}
        </div>
      </>}
    {latest && <p className="finance-card__sub">
      {capitalize(formatMonthName(latest.period))}: {formatAmount(latest.net, latest.currency)}
      {latest.paymentDate && ` · cobrado el ${formatShortDate(latest.paymentDate)}`}
      {latest.changePercent !== null && latest.previousPeriod && ` · ${formatSignedPercent(latest.changePercent)} contra ${formatMonthName(latest.previousPeriod)}`}.
    </p>}
    {salary.dollarError && <p className="finance-warn-text finance-small">Sin cotización histórica: {salary.dollarError}</p>}
  </section>
}

function AverageCard({ data }: { data: FinanceSalarySavings }) {
  const { salary } = data
  return <section className="finance-card finance-card--compact" aria-label="Promedio mensual">
    <span className="finance-card__sub">Promedio mensual · {salary.bars.length} {salary.bars.length === 1 ? 'mes' : 'meses'}</span>
    <span className="finance-figure finance-figure--md">{salary.averageArs === null ? '—' : formatAmount(salary.averageArs, 'ARS')}</span>
    {salary.averageUsd !== null && <span className="finance-soft-text">≈ US$ {salary.averageUsd.toLocaleString('es-AR', { minimumFractionDigits: 2, maximumFractionDigits: 2 })} al oficial de cada cobro</span>}
  </section>
}

function InflationCard({ data }: { data: FinanceSalarySavings }) {
  const { inflation } = data
  if (!inflation) {
    return <section className="finance-card finance-card--compact" aria-label="Contra la inflación">
      <span className="finance-card__sub">Contra la inflación</span>
      <p className="finance-soft-text">{data.inflationError ?? 'Hace falta el sueldo del mismo mes del año pasado y el IPC de esos doce meses para comparar.'}</p>
    </section>
  }
  const wins = inflation.arsVsIpc >= 0
  const points = (value: number) => `${value > 0 ? '+' : value < 0 ? '−' : ''}${Math.abs(value).toLocaleString('es-AR', { maximumFractionDigits: 1 })} pp`
  return <section className="finance-card finance-card--compact" aria-label="Contra la inflación">
    <span className="finance-card__sub">Contra la inflación · {formatShortMonthYear(inflation.from)} → {formatShortMonthYear(inflation.to)}</span>
    <div className="finance-inline-figure"><span className={`finance-figure finance-figure--md ${wins ? 'finance-figure--teal' : 'finance-warn-text'}`}>{points(inflation.arsVsIpc)}</span><span className="finance-soft-text">{wins ? 'le gana' : 'le pierde'}</span></div>
    <p className="finance-soft-text">El sueldo en pesos {inflation.arsChange >= 0 ? 'subió' : 'bajó'} <strong>{formatPercent(Math.abs(inflation.arsChange))}</strong> en un año, {wins ? 'más' : 'menos'} que los precios ({formatPercent(inflation.ipc)}).</p>
    <p className="finance-soft-text finance-card__foot">Medido en dólares oficiales {inflation.usdChange >= 0 ? 'subió' : 'bajó'} <strong>{formatPercent(Math.abs(inflation.usdChange), 2)}</strong>: {Math.abs(inflation.usdVsIpc).toLocaleString('es-AR', { maximumFractionDigits: 1 })} pp {inflation.usdVsIpc >= 0 ? 'por encima' : 'por debajo'} de la inflación.</p>
  </section>
}

// ---------------------------------------------------------------------------
// Shares of the salary

const SHARE_TITLES: Record<SalaryShareKey, string> = { cards: 'Tarjetas', services: 'Servicios aparte de la tarjeta', savings: 'Ahorro' }
const SHARE_EMPTY: Record<SalaryShareKey, string> = {
  cards: 'Todavía no vence ningún resumen este mes.',
  services: 'Todavía no hay pagos de servicios fuera de la tarjeta. Mandá una factura o un comprobante al asistente.',
  savings: 'Todavía no hay ahorro este mes.',
}
const SHARE_HISTORY: Record<SalaryShareKey, string> = {
  cards: 'El historial se completa a medida que cargues resúmenes.',
  services: 'El historial se completa a medida que cargues pagos.',
  savings: 'El historial se completa a medida que registres ahorro.',
}

function ShareCard({ share }: { share: SalaryShare }) {
  const maximum = Math.max(0, ...share.values.map((value) => value ?? 0))
  const first = share.periods[0]
  const last = share.periods[share.periods.length - 1]
  const filled = share.values.filter((value) => value !== null).length
  return <section className={`finance-card finance-share finance-share--${share.key}`} aria-labelledby={`finance-share-${share.key}`}>
    <div className="finance-card__head finance-card__head--baseline">
      <h3 id={`finance-share-${share.key}`}>{SHARE_TITLES[share.key]}</h3>
      <span className={`finance-figure finance-figure--md${share.current === null ? ' finance-muted' : ' finance-share__value'}`}>{formatPercent(share.current)}</span>
    </div>
    <div className="finance-share__cells" role="img" aria-label={share.periods.map((period, index) => `${formatShortMonthYear(period)}: ${formatPercent(share.values[index])}`).join(', ')}>
      {share.values.map((value, index) => <div key={share.periods[index]} className={value === null ? 'finance-share__cell is-empty' : 'finance-share__cell'} title={`${formatShortMonthYear(share.periods[index])}: ${formatPercent(value)}`}>
        {value !== null && <div style={{ height: `${maximum > 0 ? Math.max(12, (value / maximum) * 100) : 100}%` }} />}
      </div>)}
    </div>
    {first && last && <div className="finance-split-row finance-card__sub finance-small"><span>{formatShortMonthYear(first)}</span><span>{formatShortMonthYear(last)}</span></div>}
    <p className="finance-soft-text finance-small">
      {share.amount && share.salary
        ? `${formatMoney(share.amount)} de ${formatMoney(share.salary)}.${filled <= 1 ? ` ${SHARE_HISTORY[share.key]}` : ''}`
        : SHARE_EMPTY[share.key]}
    </p>
  </section>
}

// ---------------------------------------------------------------------------
// Dollar quotes

function QuotesCard({ data }: { data: FinanceSalarySavings }) {
  const refresh = () => notifyFinanceDataChanged()
  return <section className="finance-card finance-span-12" aria-labelledby="finance-quotes-title">
    <div className="finance-card__head">
      <div>
        <h2 id="finance-quotes-title">Cotización del dólar</h2>
        <p className="finance-card__sub">Brecha medida contra la venta del oficial.</p>
      </div>
      <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={refresh} aria-label="Actualizar cotizaciones"><RefreshCw size={14} aria-hidden="true" />Actualizar</button>
    </div>
    {data.quotesError && <p className="finance-warn-text finance-small">{data.quotesError}</p>}
    <div className="finance-quotes">
      {data.quotes.map((quote) => <QuoteTile key={quote.kind} quote={quote} />)}
      {data.lastPurchase && <div className="finance-quote finance-quote--mine">
        <div className="finance-split-row"><strong>Tu última compra</strong><span className="finance-small">{formatShortDate(data.lastPurchase.date)}</span></div>
        <div className="finance-quote__price"><span className="finance-mono-label">Pagaste por dólar</span><strong>{formatAmount(data.lastPurchase.rate, 'ARS')}</strong></div>
        <p className="finance-quote__foot">{lastPurchaseComparison(data.lastPurchase)}</p>
      </div>}
    </div>
  </section>
}

function QuoteTile({ quote }: { quote: QuoteCard }) {
  return <div className="finance-quote">
    <div className="finance-split-row">
      <strong>{quote.name}</strong>
      {quote.gapPercent === null ? <span className="finance-card__sub finance-small">referencia</span> : <span className="finance-warn-text finance-small">{formatSignedPercent(quote.gapPercent)} sobre el oficial</span>}
    </div>
    <div className="finance-quote__prices">
      <div className="finance-quote__price"><span className="finance-mono-label">Compra</span><strong>{formatAmount(quote.buy, 'ARS')}</strong></div>
      <div className="finance-quote__price"><span className="finance-mono-label">Venta</span><strong>{formatAmount(quote.sell, 'ARS')}</strong></div>
    </div>
    <div className="finance-split-row finance-quote__foot"><span>Diferencia compra-venta {formatAmount(quote.spread, 'ARS', 0)}</span><span>{formatUpdatedAt(quote.updatedAt)}</span></div>
  </div>
}

// ---------------------------------------------------------------------------
// Savings reserves

function ReserveCard({ reserve }: { reserve: ReserveView }) {
  const entry = useFinanceEntry()
  const change = Number(reserve.monthChange)
  const missing = missingReserveMovements(reserve.totals)
  return <section className="finance-card finance-span-12 finance-reserve" aria-labelledby={`finance-reserve-${reserve.id}`}>
    <div className="finance-card__head">
      <div>
        <h2 id={`finance-reserve-${reserve.id}`}>{reserve.name}</h2>
        <span className="finance-card__sub">En {reserve.currency === 'USD' ? 'dólares' : 'pesos'} · es lo registrado, no el saldo de una cuenta</span>
        {entry && <div className="finance-card__actions">
          <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry.openEntry({ kind: 'savings-movement', prefill: { reserveId: reserve.id } })}>Nuevo movimiento</button>
          <button type="button" className="finance-button finance-button--quiet-outline finance-button--small" onClick={() => entry.openEntry({ kind: 'reserve', id: reserve.id })}>Editar reserva</button>
        </div>}
      </div>
      <div className="finance-reserve__total">
        <div className="finance-figure finance-figure--xl">{formatAmount(reserve.balance, reserve.currency)}</div>
        {change !== 0 && <div className={change > 0 ? 'finance-teal finance-small' : 'finance-warn-text finance-small'}>{change > 0 ? '+' : '−'} {formatAmount(Math.abs(change), reserve.currency, 0)} este mes</div>}
        {reserve.valuation.length > 0 && <div className="finance-card__sub finance-small">≈ {reserve.valuation.map((item) => `${formatAmount(item.amount, 'ARS', 0)} al ${item.kind} compra`).join(' · ')}</div>}
      </div>
    </div>
    <div className="finance-reserve__chips">
      {reserve.totals.map((total) => <span key={total.kind} className="finance-pill finance-pill--paid">{MOVEMENT_TYPES[total.kind]?.total ?? total.kind} <strong>{formatAmount(total.amount, reserve.currency, 0)}</strong></span>)}
      {missing.length > 0 && <span className="finance-pill">{reserve.totals.length === 0 ? 'Sin movimientos este mes' : `Sin ${joinSpanish(missing)}`}</span>}
    </div>
    {reserve.movements.length > 0 && <div className="finance-table-scroll">
      <table className="finance-table">
        <thead><tr><th scope="col">Fecha</th><th scope="col">Motivo</th><th scope="col">Tipo</th><th scope="col" className="is-number">Costo en pesos</th><th scope="col" className="is-number">Importe</th></tr></thead>
        <tbody>{reserve.movements.map((movement) => <tr key={movement.id}>
          <td className="finance-muted">{formatShortDate(movement.date)}</td>
          <td>{movement.reason || '—'}</td>
          <td>{MOVEMENT_TYPES[movement.movementType]?.row ?? movement.movementType}</td>
          <td className="is-number finance-soft-text">{movement.cost ? `${formatMoney(movement.cost, 0)}${movement.rate ? ` · a ${formatAmount(movement.rate, movement.cost.currency, 0)}` : ''}` : '—'}</td>
          <td className={`is-number finance-strong ${reserveMovementAdds(movement.movementType) ? 'finance-teal' : ''}`}>{reserveMovementAdds(movement.movementType) ? '+' : '−'} {formatAmount(movement.amount, movement.currency)}</td>
        </tr>)}</tbody>
      </table>
    </div>}
  </section>
}
