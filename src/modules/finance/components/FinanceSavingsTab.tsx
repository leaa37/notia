import { useCallback } from 'react'
import { ArrowDownLeft, ArrowUpRight, Plus } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceSavings } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatExact, formatPercent, formatShortDate, formatUpdatedAt } from '../engines/financeFormat'
import type { FinanceSavings, ReserveRow, SavingsRow } from '../types/financeScreen'
import { useFinanceEntry } from './entry/financeEntryContext'
import { FinanceBar, FinanceCard, FinanceStatus } from './FinanceParts'

/** «Ahorro»: the reserves, what each holds, the dollar and the movements. Nothing else. */
export function FinanceSavingsTab({ library }: { library: NotiaLibrary }) {
  const load = useCallback(() => getFinanceSavings(library), [library])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo leer el ahorro.')
  const entry = useFinanceEntry()
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  return <div className="finance-savings">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    <Totals data={data} />
    <div className="finance-savings__actions">
      <button type="button" className="finance-button finance-button--primary" onClick={() => entry?.openEntry({ kind: 'savings', prefill: { type: 'deposit' } })}><ArrowDownLeft size={16} aria-hidden="true" />Guardar</button>
      <button type="button" className="finance-button finance-button--ghost" onClick={() => entry?.openEntry({ kind: 'savings', prefill: { type: 'withdrawal' } })}><ArrowUpRight size={16} aria-hidden="true" />Sacar</button>
      <button type="button" className="finance-button finance-button--ghost" onClick={() => entry?.openEntry({ kind: 'exchange', prefill: { type: 'buy' } })}>Comprar dólares</button>
      <button type="button" className="finance-button finance-button--ghost" onClick={() => entry?.openEntry({ kind: 'exchange', prefill: { type: 'sell' } })}>Vender dólares</button>
    </div>
    <div className="finance-grid">
      <FinanceCard className="finance-span-7" title="Reservas" action={<button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={() => entry?.openEntry({ kind: 'reserve' })}><Plus size={14} aria-hidden="true" />Reserva</button>}>
        {data.reserves.length === 0 ? <p className="finance-empty">Sin reservas.</p> : <ul className="finance-pay-list" role="list">
          {data.reserves.map((reserve) => <Reserve key={reserve.id} reserve={reserve} />)}
        </ul>}
      </FinanceCard>
      <Quotes data={data} />
      <FinanceCard className="finance-span-12" title="Movimientos de ahorro">
        {data.movements.length === 0 ? <p className="finance-empty">Todavía no guardaste nada.</p> : <ul className="finance-movements" role="list">
          {data.movements.map((row) => <Movement key={row.id} row={row} />)}
        </ul>}
      </FinanceCard>
    </div>
  </div>
}

function Totals({ data }: { data: FinanceSavings }) {
  return <div className="finance-split finance-card">
    {data.totals.map((total) => <div key={total.currency}>
      <span className="finance-mono-label">{total.currency === 'USD' ? 'Ahorrado en dólares' : 'Ahorrado en pesos'}</span>
      <strong className="finance-figure finance-figure--hero">{formatAmount(total.balance, total.currency, 0)}</strong>
      <span className="finance-card__sub">
        {Number(total.savedThisMonth) === 0 ? 'Nada este mes' : `${Number(total.savedThisMonth) > 0 ? '+' : ''}${formatAmount(total.savedThisMonth, total.currency, 0)} este mes`}
        {total.currency === 'USD' && data.dollarsInPesos ? ` · ${formatAmount(data.dollarsInPesos.pesos, 'ARS', 0)} al ${data.dollarsInPesos.quote.toLowerCase()}` : ''}
      </span>
    </div>)}
  </div>
}

function Reserve({ reserve }: { reserve: ReserveRow }) {
  const entry = useFinanceEntry()
  return <li className="finance-pay-row">
    <button type="button" className="finance-pay-row__main finance-reserve" onClick={() => entry?.openEntry({ kind: 'reserve', id: reserve.id })}>
      <span className="finance-pay-row__name">{reserve.name}</span>
      {reserve.goal && reserve.goalPercent !== null && <>
        <FinanceBar percent={reserve.goalPercent} label={`${formatPercent(reserve.goalPercent, 0)} del objetivo`} />
        <span className="finance-pay-row__sub">{formatPercent(reserve.goalPercent, 0)} de {formatAmount(reserve.goal, reserve.currency, 0)}</span>
      </>}
      {Number(reserve.savedThisMonth) !== 0 && <span className="finance-pay-row__sub">{Number(reserve.savedThisMonth) > 0 ? '+' : ''}{formatAmount(reserve.savedThisMonth, reserve.currency, 0)} este mes</span>}
    </button>
    <span className="finance-pay-row__amount">{formatAmount(reserve.balance, reserve.currency, 0)}</span>
    <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry?.openEntry({ kind: 'savings', prefill: { reserveId: reserve.id, type: 'deposit' } })}>Guardar</button>
  </li>
}

function Quotes({ data }: { data: FinanceSavings }) {
  const oficial = data.quotes.find((quote) => quote.kind === 'oficial')
  return <FinanceCard className="finance-span-5" title="Dólar" sub={oficial ? `Actualizado ${formatUpdatedAt(oficial.updatedAt)}` : undefined}>
    {data.quotesError ? <p className="finance-empty">No se pudieron traer las cotizaciones.</p> : <table className="finance-table">
      <thead><tr><th scope="col">Tipo</th><th scope="col" className="is-number">Compra</th><th scope="col" className="is-number">Venta</th></tr></thead>
      <tbody>{data.quotes.map((quote) => <tr key={quote.kind}><td>{quote.name}</td><td className="is-number">{formatAmount(quote.buy, 'ARS', 0)}</td><td className="is-number">{formatAmount(quote.sell, 'ARS', 0)}</td></tr>)}</tbody>
    </table>}
  </FinanceCard>
}

function Movement({ row }: { row: SavingsRow }) {
  const entry = useFinanceEntry()
  const deposit = row.direction === 'deposit'
  return <li>
    <button type="button" className="finance-movement" onClick={() => entry?.openEntry({ kind: row.kind, id: row.id })}>
      <span className="finance-movement__date">{formatShortDate(row.date)}</span>
      <span className="finance-movement__main">
        <span className="finance-movement__name">{row.title}</span>
        <span className="finance-movement__meta">{[row.reserve, row.pesos ? `${formatAmount(row.pesos, 'ARS', 0)}` : null, row.note || null].filter(Boolean).join(' · ')}</span>
      </span>
      <span className={`finance-movement__amount${deposit ? ' finance-movement__amount--income' : ''}`}>{deposit ? '+ ' : '− '}{formatExact(row.amount, row.currency)}</span>
    </button>
  </li>
}
