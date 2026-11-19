import { useCallback, useEffect, useState } from 'react'
import { Download, Pencil, Search } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getFinanceMovements } from '../services/financeService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { formatAmount, formatLongDate, formatMoney, formatMoneyList, formatShortDate, plural } from '../engines/financeFormat'
import { movementAmount, movementKindLabel, movementMeta, movementStatusLabel, movementTone } from '../engines/financeMovementText'
import type { FinanceMovements, MovementGroup, MovementRow } from '../types/financeScreen'
import { FinanceStatus } from './FinanceStatus'
import { FinanceHistory } from './FinanceHistory'
import { useFinanceEntry } from './entry/financeEntryContext'
import { PhoneMovements } from './phone/PhoneMovements'

const SEARCH_DELAY_MS = 250

interface Props {
  library: NotiaLibrary
  month: string
  onOpenChat: (prompt: string) => void
  /** The phone board of the canvas. */
  phone: boolean
}

type Scope = 'month' | 'history'

export function FinanceMovementsTab(props: Props) {
  const entry = useFinanceEntry()
  const [scope, setScope] = useState<Scope>('month')
  return <>
    <div className={props.phone ? 'finance-scope finance-scope--phone' : 'finance-scope'}>
      <div className="finance-segmented" role="group" aria-label="Qué movimientos">
        <button type="button" aria-pressed={scope === 'month'} onClick={() => setScope('month')}>Este mes</button>
        <button type="button" aria-pressed={scope === 'history'} onClick={() => setScope('history')}>Todo el historial</button>
      </div>
      <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={() => entry?.openExport(props.month)}>
        <Download size={14} aria-hidden="true" />Exportar
      </button>
    </div>
    {scope === 'history' ? <FinanceHistory library={props.library} phone={props.phone} /> : <MonthMovements {...props} />}
  </>
}

function MonthMovements({ library, month, onOpenChat, phone }: Props) {
  const [filter, setFilter] = useState('all')
  const [search, setSearch] = useState('')
  const [appliedSearch, setAppliedSearch] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)

  useEffect(() => {
    const timer = window.setTimeout(() => setAppliedSearch(search), SEARCH_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [search])

  const load = useCallback(() => getFinanceMovements(library, { month, filter, search: appliedSearch }), [appliedSearch, filter, library, month])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudieron cargar los movimientos.')
  const rows = data?.groups.flatMap((group) => group.rows) ?? []
  const selected = rows.find((row) => row.id === selectedId) ?? rows[0] ?? null
  const filtered = appliedSearch.trim() !== '' || (data?.filter ?? filter) !== 'all'

  if (phone) {
    return <PhoneMovements
      data={data}
      filter={data?.filter ?? filter}
      search={search}
      filtered={filtered}
      status={!data || error ? <FinanceStatus isLoading={isLoading && !data} error={error} onRetry={reload} /> : null}
      summary={data ? summary(data) : ''}
      onSearch={setSearch}
      onFilter={setFilter}
      onOpenChat={onOpenChat}
    />
  }

  return <div className="finance-movements">
    <section className="finance-movements__list" aria-label="Movimientos del mes">
      <div className="finance-filters">
        <label className="finance-search">
          <Search size={16} aria-hidden="true" />
          <span className="finance-visually-hidden">Buscar movimiento</span>
          <input type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Buscar: Movistar, MERPAGO…" />
        </label>
        {(data?.chips ?? []).map((chip) => <button key={chip.id} type="button" className="finance-chip" aria-pressed={(data?.filter ?? filter) === chip.id} onClick={() => setFilter(chip.id)}>
          {chip.label} <span className="finance-chip__count">{chip.count}</span>
        </button>)}
      </div>
      {!data ? <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} /> : <>
        {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
        <p className="finance-card__sub" aria-live="polite">{summary(data)}</p>
        {data.groups.map((group) => <MovementGroupCard key={group.accountId} group={group} selectedId={selected?.id ?? null} onPick={setSelectedId} />)}
        {data.groups.length === 0 && <p className="finance-empty finance-empty--boxed">{filtered ? 'No hay movimientos con ese filtro.' : 'No hay movimientos en este mes.'}</p>}
      </>}
    </section>
    <aside className="finance-detail" aria-label="Detalle del movimiento">
      {selected ? <MovementDetail row={selected} prompt={data?.changePrompts[selected.id] ?? null} onOpenChat={onOpenChat} /> : <p className="finance-card__sub">Elegí un movimiento para ver su detalle.</p>}
    </aside>
  </div>
}

function summary(data: FinanceMovements): string {
  const { expenseCount, expenseTotals, exchangeCount, incomeCount } = data.summary
  const parts = [`${plural(expenseCount, 'gasto')} · ${formatMoneyList(expenseTotals)}`]
  if (exchangeCount > 0) parts.push(`más ${plural(exchangeCount, 'cambio de moneda', 'cambios de moneda')}`)
  if (incomeCount > 0) parts.push(`más ${plural(incomeCount, 'ingreso')}`)
  return parts.join(' · ')
}

function MovementGroupCard({ group, selectedId, onPick }: { group: MovementGroup; selectedId: string | null; onPick: (id: string) => void }) {
  const total = group.totals.length > 0
    ? formatMoneyList(group.totals)
    : group.savings.length > 0 ? group.savings.map((money) => `+ ${formatMoney(money)}`).join(' · ') : '—'
  return <div className="finance-group">
    <div className="finance-group__head">
      <span className="finance-group__title">
        <span className="finance-badge finance-badge--small">{group.badge}</span>
        <span className="finance-group__name">{group.name}</span>
        <span className="finance-card__sub">{group.statement ? `Resumen de ${formatMoney(group.statement.total)} · vence ${formatShortDate(group.statement.dueDate)}` : group.kindLabel}</span>
      </span>
      <strong className={group.totals.length === 0 && group.savings.length > 0 ? 'finance-teal' : undefined}>{total}</strong>
    </div>
    {group.rows.map((row) => {
      const tone = movementTone(row)
      return <button key={row.id} type="button" className="finance-row" aria-pressed={row.id === selectedId} onClick={() => onPick(row.id)}>
        <span className="finance-row__date">{formatShortDate(row.purchaseDate)}</span>
        <span className="finance-row__main">
          <span className={`finance-row__desc${tone === 'off' ? ' finance-tone--off' : ''}`}>{row.description || movementKindLabel(row.kind)}</span>
          <span className={row.flagged ? 'finance-row__meta finance-warn-text' : 'finance-row__meta'}>{movementMeta(row)}</span>
        </span>
        <span className={`finance-row__amount finance-tone--${tone}`}>{movementAmount(row)}</span>
      </button>
    })}
  </div>
}

function MovementDetail({ row, prompt, onOpenChat }: { row: MovementRow; prompt: string | null; onOpenChat: (prompt: string) => void }) {
  const entry = useFinanceEntry()
  const secondary = row.exchange
    ? `Pagaste ${formatAmount(row.amount, row.currency)}${row.exchange.rate ? ` · ${formatAmount(row.exchange.rate, row.currency, 0)} por ${row.exchange.currency === 'USD' ? 'dólar' : row.exchange.currency}` : ''}`
    : `${movementKindLabel(row.kind)} en ${row.currency === 'ARS' ? 'pesos' : 'dólares'}`
  const category = row.exchange ? 'Ahorro' : row.categoryName ?? (row.uncategorized ? 'Sin categoría' : '—')
  return <>
    <div className="finance-detail__top">
      <span className="finance-mono-label">Detalle</span>
      {entry && <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry.openEntry({ kind: 'movement', id: row.id })}>
        <Pencil size={14} aria-hidden="true" />Editar
      </button>}
    </div>
    <div className="finance-detail__head">
      <span className="finance-detail__desc">{row.description || movementKindLabel(row.kind)}</span>
      <span className={`finance-figure finance-figure--xl finance-tone--${movementTone(row)}`}>{movementAmount(row)}</span>
      <span className="finance-card__sub">{secondary}</span>
    </div>
    {row.note && <p className="finance-note">{row.note}</p>}
    <dl className="finance-detail__facts">
      <dt>Compra</dt><dd>{formatLongDate(row.purchaseDate)}</dd>
      <dt>Cuenta en</dt><dd>{formatShortDate(row.effectiveDate)}{row.countsOnStatementDue && ' (vencimiento del resumen)'}</dd>
      <dt>Cuenta</dt><dd>{row.accountName}</dd>
      <dt>Categoría</dt><dd>{category}</dd>
      <dt>Servicio</dt><dd>{row.serviceName ?? '—'}</dd>
      <dt>Cargado desde</dt><dd>{row.origin}</dd>
      <dt>Estado</dt><dd>{movementStatusLabel(row.status)}</dd>
    </dl>
    {prompt && <div className="finance-detail__foot">
      <button type="button" className="finance-button finance-button--outline" onClick={() => onOpenChat(prompt)}>Pedir un cambio en el chat</button>
      <p className="finance-card__sub">Abre el chat con este movimiento citado. El asistente propone el cambio y lo confirmás ahí.</p>
    </div>}
  </>
}
