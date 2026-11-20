import { useCallback, useEffect, useState } from 'react'
import { Download, History, Search } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceMovements } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import type { FilterOption, MovementFilter } from '../types/financeScreen'
import { useFinanceEntry } from './entry/financeEntryContext'
import { FinanceStatus, MoneyText, MovementList } from './FinanceParts'
import { FinanceHistory } from './FinanceHistory'

const EMPTY_FILTER: MovementFilter = { kind: '', categoryId: '', accountId: '', search: '' }

/** «Movimientos»: the month (or the whole history) with Rust's filters. */
export function FinanceMovementsTab({ library, month }: { library: NotiaLibrary; month: string }) {
  const [history, setHistory] = useState(false)
  const [filter, setFilter] = useState<MovementFilter>(EMPTY_FILTER)
  const [search, setSearch] = useState('')
  const entry = useFinanceEntry()

  // The search waits for the typing to stop.
  useEffect(() => {
    const timer = window.setTimeout(() => setFilter((current) => (current.search === search ? current : { ...current, search })), 250)
    return () => window.clearTimeout(timer)
  }, [search])

  const load = useCallback(() => getFinanceMovements(library, month, filter), [library, month, filter])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudieron leer los movimientos.')

  return <div className="finance-movements-tab">
    <div className="finance-toolbar">
      <div className="finance-chips" role="group" aria-label="Tipo">
        {([['', 'Todos'], ['expense', 'Gastos'], ['income', 'Ingresos']] as const).map(([kind, label]) => <button key={kind} type="button" className="finance-chip" aria-pressed={filter.kind === kind} onClick={() => setFilter((current) => ({ ...current, kind }))}>{label}</button>)}
      </div>
      <label className="finance-search">
        <Search size={16} aria-hidden="true" />
        <span className="finance-visually-hidden">Buscar</span>
        <input type="search" value={search} placeholder="Buscar por nombre, categoría o importe" onChange={(event) => setSearch(event.target.value)} />
      </label>
      {data && <Picker label="Categoría" value={filter.categoryId} options={[...data.categories, { id: 'none', name: 'Sin categoría' }]} onChange={(categoryId) => setFilter((current) => ({ ...current, categoryId }))} />}
      {data && data.accounts.length > 0 && <Picker label="Cuenta" value={filter.accountId} options={data.accounts} onChange={(accountId) => setFilter((current) => ({ ...current, accountId }))} />}
      <div className="finance-toolbar__end">
        <button type="button" className="finance-button finance-button--ghost finance-button--small" aria-pressed={history} onClick={() => setHistory((value) => !value)}><History size={14} aria-hidden="true" />{history ? 'Solo este mes' : 'Todo el historial'}</button>
        <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={() => entry?.openExport(month)}><Download size={14} aria-hidden="true" />Exportar</button>
      </div>
    </div>
    {history ? <FinanceHistory library={library} filter={filter} /> : !data ? <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} /> : <>
      {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
      <div className="finance-totals">
        <span>Gastos <strong><MoneyText list={data.expenses} empty="$ 0" decimals={2} /></strong></span>
        <span>Ingresos <strong className="finance-teal"><MoneyText list={data.income} empty="$ 0" decimals={2} /></strong></span>
        <span className="finance-muted">{data.rows.length} {data.rows.length === 1 ? 'movimiento' : 'movimientos'}</span>
      </div>
      <section className="finance-card finance-card--list">
        <MovementList rows={data.rows} empty={filter.kind || filter.categoryId || filter.accountId || filter.search ? 'Nada coincide con el filtro.' : 'Sin movimientos este mes.'} />
      </section>
    </>}
  </div>
}

function Picker({ label, value, options, onChange }: { label: string; value: string; options: FilterOption[]; onChange: (value: string) => void }) {
  return <label className="finance-picker">
    <span className="finance-visually-hidden">{label}</span>
    <select value={value} onChange={(event) => onChange(event.target.value)}>
      <option value="">{label}: todas</option>
      {options.map((option) => <option key={option.id} value={option.id}>{option.name}</option>)}
    </select>
  </label>
}
