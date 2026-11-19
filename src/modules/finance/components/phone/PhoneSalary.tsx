import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { notifyFinanceDataChanged } from '../../services/financeDataEvents'
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
} from '../../engines/financeFormat'
import {
  capitalize,
  describeQuoteDifference,
  joinSpanish,
  missingReserveMovements,
  RESERVE_MOVEMENT_TYPES,
  reserveMovementAdds,
} from '../../engines/financeSalaryText'
import type { FinanceSalarySavings, QuoteCard, ReserveView, SalaryShare, SalaryShareKey } from '../../types/financeScreen'
import { useFinanceEntry } from '../entry/financeEntryContext'

/*
 * Sueldo y ahorro in the space of a phone: the canvas board «Teléfono ·
 * Sueldo y ahorro». Every figure comes from Rust (`finance_salary_savings`).
 */

const PHONE_BARS = 6
const BAR_MAX_HEIGHT = 148

export function PhoneSalary({ data }: { data: FinanceSalarySavings }) {
  return <div className="finance-phone">
    <PhoneSalaryChart data={data} />
    <div className="finance-phone-tiles">
      <AverageTile data={data} />
      <InflationTile data={data} />
    </div>
    {data.inflation && <p className="finance-phone-box">
      Medido en dólares oficiales {data.inflation.usdChange >= 0 ? 'subió' : 'bajó'} <strong>{formatPercent(Math.abs(data.inflation.usdChange), 2)}</strong>: {Math.abs(data.inflation.usdVsIpc).toLocaleString('es-AR', { maximumFractionDigits: 1 })} pp {data.inflation.usdVsIpc >= 0 ? 'por encima' : 'por debajo'} de la inflación.
    </p>}
    <PhoneShares shares={data.shares} />
    <PhoneQuotes data={data} />
    {data.reserves.map((reserve) => <PhoneReserve key={reserve.id} reserve={reserve} />)}
    {data.reserves.length === 0 && <section className="finance-phone-card"><h2>Ahorro</h2><p className="finance-phone-sub">Todavía no hay reservas de ahorro. Creá una en «Administrar» o pedíselo al asistente.</p></section>}
  </div>
}

function PhoneSalaryChart({ data }: { data: FinanceSalarySavings }) {
  const { salary } = data
  const [inDollars, setInDollars] = useState(false)
  const bars = salary.bars.slice(-PHONE_BARS)
  const dollarsAvailable = salary.bars.length > 0 && salary.bars.every((bar) => bar.usd !== null)
  const useDollars = inDollars && dollarsAvailable
  const value = (bar: { ars: number; usd: number | null }) => (useDollars ? bar.usd ?? 0 : bar.ars)
  const average = useDollars ? salary.averageUsd : salary.averageArs
  const maximum = Math.max(1, ...bars.map(value), average ?? 0)
  const label = (amount: number) => (useDollars ? formatAmount(amount, 'USD', 0).replace('USD ', '') : formatMillions(amount).replace(' M', ''))
  const latest = salary.latest
  const last = bars[bars.length - 1]
  const range = bars.length > 0 ? `${formatMonthName(bars[0].period)} a ${formatMonthName(last.period)} ${last.period.slice(0, 4)}` : ''
  return <section className="finance-phone-card" aria-labelledby="finance-phone-salary-title">
    <div className="finance-phone-row">
      <h2 id="finance-phone-salary-title">Sueldo neto</h2>
      <div className="finance-phone-segmented" role="group" aria-label="Cómo ver el sueldo">
        <button type="button" aria-pressed={!useDollars} onClick={() => setInDollars(false)}>Pesos</button>
        <button type="button" aria-pressed={useDollars} disabled={!dollarsAvailable} onClick={() => setInDollars(true)}>Dólares</button>
      </div>
    </div>
    {latest && <div className="finance-phone-stack">
      <span className="finance-phone-figure finance-phone-figure--lg">{formatAmount(latest.net, latest.currency)}</span>
      <span className="finance-phone-sub">
        {formatMonthName(latest.period)}
        {latest.paymentDate && ` · cobrado el ${formatShortDate(latest.paymentDate)}`}
        {latest.changePercent !== null && latest.previousPeriod && ` · ${formatSignedPercent(latest.changePercent)} contra ${formatMonthName(latest.previousPeriod)}`}
      </span>
    </div>}
    {bars.length === 0
      ? <p className="finance-phone-sub">Todavía no hay recibos de sueldo. Mandale uno al asistente.</p>
      : <>
        <div className="finance-phone-bars" role="img" aria-label={bars.map((bar) => `${formatShortMonthYear(bar.period)}: ${useDollars ? formatAmount(value(bar), 'USD', 0) : formatMillions(value(bar))}`).join(', ')}>
          {average !== null && <>
            <div className="finance-phone-bars__average" style={{ bottom: `${(average / maximum) * BAR_MAX_HEIGHT}px` }} />
            <span className="finance-phone-bars__average-label" style={{ bottom: `${(average / maximum) * BAR_MAX_HEIGHT + 4}px` }}>prom. {salary.bars.length} {salary.bars.length === 1 ? 'mes' : 'meses'} {useDollars ? formatAmount(average, 'USD', 0) : formatMillions(average)}</span>
          </>}
          {bars.map((bar, index) => <div key={bar.period} className={`finance-phone-bars__bar${index === bars.length - 1 ? ' is-current' : ''}`}>
            <span>{label(value(bar))}</span>
            <div style={{ height: `${Math.max(2, (value(bar) / maximum) * BAR_MAX_HEIGHT)}px` }} />
          </div>)}
        </div>
        <div className="finance-phone-bars__axis" aria-hidden="true">
          {bars.map((bar, index) => <span key={bar.period} className={index === bars.length - 1 ? 'is-current' : undefined}>{formatShortMonth(bar.period)}</span>)}
        </div>
        <span className="finance-phone-sub">{useDollars ? 'En dólares oficiales' : 'En millones de pesos'} · {range}</span>
      </>}
    {salary.dollarError && <p className="finance-warn-text finance-phone-sub">Sin cotización histórica: {salary.dollarError}</p>}
  </section>
}

function AverageTile({ data }: { data: FinanceSalarySavings }) {
  const { salary } = data
  return <section className="finance-phone-tile">
    <span>Promedio · {salary.bars.length} {salary.bars.length === 1 ? 'mes' : 'meses'}</span>
    <strong>{salary.averageArs === null ? '—' : formatAmount(salary.averageArs, 'ARS', 0)}</strong>
    <small>{salary.averageUsd !== null ? `≈ US$ ${salary.averageUsd.toLocaleString('es-AR', { maximumFractionDigits: 0 })}` : 'sin cotización'}</small>
  </section>
}

function InflationTile({ data }: { data: FinanceSalarySavings }) {
  const { inflation } = data
  if (!inflation) {
    return <section className="finance-phone-tile">
      <span>Contra la inflación</span>
      <strong>—</strong>
      <small>{data.inflationError ?? 'Hace falta el sueldo de hace un año para comparar.'}</small>
    </section>
  }
  const wins = inflation.arsVsIpc >= 0
  const points = `${inflation.arsVsIpc > 0 ? '+' : inflation.arsVsIpc < 0 ? '−' : ''}${Math.abs(inflation.arsVsIpc).toLocaleString('es-AR', { maximumFractionDigits: 1 })} pp`
  return <section className="finance-phone-tile">
    <span>Contra la inflación</span>
    <strong className={wins ? 'finance-figure--teal' : 'finance-warn-text'}>{points}</strong>
    <small>{formatSignedPercent(inflation.arsChange)} en pesos en un año</small>
  </section>
}

const SHARE_ORDER: SalaryShareKey[] = ['cards', 'savings', 'services']
const SHARE_TITLES: Record<SalaryShareKey, string> = { cards: 'Tarjetas', savings: 'Ahorro', services: 'Servicios aparte' }
const SHARE_EMPTY: Record<SalaryShareKey, string> = {
  cards: 'Todavía no vence ningún resumen este mes',
  savings: 'Todavía no hay ahorro este mes',
  services: 'Sin pagos de servicios registrados todavía',
}

function PhoneShares({ shares }: { shares: SalaryShare[] }) {
  const ordered = SHARE_ORDER.map((key) => shares.find((share) => share.key === key)).filter((share): share is SalaryShare => Boolean(share))
  const periods = ordered[0]?.periods ?? []
  return <section className="finance-phone-card finance-phone-card--list" aria-labelledby="finance-phone-shares-title">
    <div className="finance-phone-stack">
      <h2 id="finance-phone-shares-title">Qué parte del sueldo se va en…</h2>
      {periods.length > 0 && <span className="finance-phone-sub">Cada mes contra el sueldo del mes anterior · {formatShortMonthYear(periods[0])} a {formatShortMonthYear(periods[periods.length - 1])}</span>}
    </div>
    {ordered.map((share) => <div key={share.key} className={`finance-phone-share finance-phone-share--${share.key}`}>
      <div className="finance-phone-row finance-phone-row--baseline">
        <span className="finance-phone-medium">{SHARE_TITLES[share.key]}</span>
        <span className={`finance-phone-figure finance-phone-share__value${share.current === null ? ' finance-muted' : ''}`}>{share.current === null ? '—' : formatPercent(share.current)}</span>
      </div>
      <div className="finance-phone-share__cells" role="img" aria-label={share.periods.map((period, index) => `${formatShortMonthYear(period)}: ${formatPercent(share.values[index])}`).join(', ')}>
        {share.values.map((value, index) => <span key={share.periods[index]} className={value === null ? 'is-empty' : undefined} title={`${formatShortMonthYear(share.periods[index])}: ${formatPercent(value)}`} />)}
      </div>
      <span className="finance-phone-sub">{share.amount && share.salary ? `${formatMoney(share.amount)} de ${formatMoney(share.salary)}` : SHARE_EMPTY[share.key]}</span>
    </div>)}
  </section>
}

function PhoneQuotes({ data }: { data: FinanceSalarySavings }) {
  const purchase = data.lastPurchase
  return <section className="finance-phone-card" aria-labelledby="finance-phone-quotes-title">
    <div className="finance-phone-row">
      <div className="finance-phone-stack">
        <h2 id="finance-phone-quotes-title">Cotización del dólar</h2>
        <span className="finance-phone-sub">Brecha contra la venta del oficial</span>
      </div>
      <button type="button" className="finance-phone-icon" aria-label="Actualizar cotizaciones" onClick={() => notifyFinanceDataChanged()}><RefreshCw size={16} /></button>
    </div>
    {data.quotesError && <p className="finance-warn-text finance-phone-sub">{data.quotesError}</p>}
    <div className="finance-phone-quotes">
      {data.quotes.map((quote) => <PhoneQuote key={quote.kind} quote={quote} />)}
      {purchase && <div className="finance-phone-quote finance-phone-quote--mine">
        <span className="finance-phone-quote__name">Tu compra</span>
        <span className="finance-phone-figure">{formatAmount(purchase.rate, 'ARS', 0)}</span>
        <span className="finance-phone-quote__note">{formatShortDate(purchase.date)}{purchase.vsOfficial !== null && ` · ${describeQuoteDifference(purchase.vsOfficial, 'el oficial')}`}</span>
      </div>}
    </div>
  </section>
}

function PhoneQuote({ quote }: { quote: QuoteCard }) {
  return <div className="finance-phone-quote">
    <div className="finance-phone-row finance-phone-row--baseline">
      <span className="finance-phone-quote__name">{quote.name}</span>
      {quote.gapPercent === null ? <span className="finance-phone-tiny">referencia</span> : <span className="finance-phone-tiny finance-warn-text">{formatSignedPercent(quote.gapPercent)}</span>}
    </div>
    <div className="finance-phone-row finance-phone-small"><span className="finance-muted">Compra</span><strong>{formatAmount(quote.buy, 'ARS', 0)}</strong></div>
    <div className="finance-phone-row finance-phone-small"><span className="finance-muted">Venta</span><strong>{formatAmount(quote.sell, 'ARS', 0)}</strong></div>
    <span className="finance-phone-tiny">{formatUpdatedAt(quote.updatedAt)}</span>
  </div>
}

function PhoneReserve({ reserve }: { reserve: ReserveView }) {
  const entry = useFinanceEntry()
  const change = Number(reserve.monthChange)
  const missing = missingReserveMovements(reserve.totals)
  return <section className="finance-phone-card" aria-labelledby={`finance-phone-reserve-${reserve.id}`}>
    <div className="finance-phone-stack">
      <h2 id={`finance-phone-reserve-${reserve.id}`}>{reserve.name}</h2>
      <span className="finance-phone-sub">Lo registrado, no el saldo de una cuenta</span>
    </div>
    {entry && <div className="finance-phone-actions">
      <button type="button" className="finance-phone-button finance-phone-button--accent" onClick={() => entry.openEntry({ kind: 'savings-movement', prefill: { reserveId: reserve.id } })}>Nuevo movimiento</button>
      <button type="button" className="finance-phone-button" onClick={() => entry.openEntry({ kind: 'reserve', id: reserve.id })}>Editar</button>
    </div>}
    <div className="finance-phone-stack">
      <span className="finance-phone-figure finance-phone-figure--xl">{formatAmount(reserve.balance, reserve.currency)}</span>
      {change !== 0 && <span className={change > 0 ? 'finance-teal finance-phone-small' : 'finance-warn-text finance-phone-small'}>{change > 0 ? '+' : '−'} {formatAmount(Math.abs(change), reserve.currency, 0)} este mes</span>}
      {reserve.valuation.length > 0 && <span className="finance-phone-sub">≈ {reserve.valuation.map((item) => `${formatAmount(item.amount, 'ARS', 0)} al ${item.kind} compra`).join(' · ')}</span>}
    </div>
    <div className="finance-phone-pills">
      {reserve.totals.map((total) => <span key={total.kind} className="finance-pill finance-pill--paid">{RESERVE_MOVEMENT_TYPES[total.kind]?.total ?? total.kind} {formatAmount(total.amount, reserve.currency, 0)}</span>)}
      {missing.length > 0 && <span className="finance-pill">{reserve.totals.length === 0 ? 'Sin movimientos este mes' : capitalize(`sin ${joinSpanish(missing)}`)}</span>}
    </div>
    {reserve.movements.map((movement) => {
      const adds = reserveMovementAdds(movement.movementType)
      const kind = RESERVE_MOVEMENT_TYPES[movement.movementType]?.row ?? movement.movementType
      return <div key={movement.id} className="finance-phone-line finance-phone-line--top">
        <span className="finance-phone-line__main">
          <span>{movement.reason || kind}</span>
          <span className="finance-phone-sub">{formatShortDate(movement.date)} · {kind}{movement.cost && ` · ${formatMoney(movement.cost, 0)}`}</span>
        </span>
        <strong className={`finance-phone-nowrap${adds ? ' finance-teal' : ''}`}>{adds ? '+' : '−'} {formatAmount(movement.amount, movement.currency, 0)}</strong>
      </div>
    })}
  </section>
}
