import { useState, type ReactNode } from 'react'
import { Search } from 'lucide-react'
import { formatLongDate, formatMoney, formatMoneyList, formatShortDate } from '../../engines/financeFormat'
import { movementAmount, movementKindLabel, movementMeta, movementStatusLabel, movementTone } from '../../engines/financeMovementText'
import { useFinanceEntry } from '../entry/financeEntryContext'
import type { FinanceMovements, MovementGroup, MovementRow } from '../../types/financeScreen'

/*
 * Movimientos in the space of a phone: the canvas board «Teléfono ·
 * Movimientos». A row opens its detail in place. The filter, the search and
 * every figure come from Rust (`finance_movements`).
 */

interface Props {
  data: FinanceMovements | null
  filter: string
  search: string
  filtered: boolean
  /** Loading or error, from the tab. */
  status: ReactNode
  summary: string
  onSearch: (search: string) => void
  onFilter: (filter: string) => void
  onOpenChat: (prompt: string) => void
}

export function PhoneMovements({ data, filter, search, filtered, status, summary, onSearch, onFilter, onOpenChat }: Props) {
  const [openId, setOpenId] = useState<string | null>(null)
  return <div className="finance-phone">
    <label className="finance-phone-search">
      <Search size={16} aria-hidden="true" />
      <span className="finance-visually-hidden">Buscar movimiento</span>
      <input type="search" value={search} onChange={(event) => onSearch(event.target.value)} placeholder="Buscar: Movistar, MERPAGO…" />
    </label>
    {data && data.chips.length > 0 && <div className="finance-phone-chips" role="group" aria-label="Filtrar movimientos">
      {data.chips.map((chip) => <button key={chip.id} type="button" className="finance-phone-chip" aria-pressed={filter === chip.id} onClick={() => onFilter(chip.id)}>
        {chip.label} <span className="finance-phone-chip__count">{chip.count}</span>
      </button>)}
    </div>}
    {status}
    {data && <>
      <p className="finance-phone-sub finance-phone-small" aria-live="polite">{summary}</p>
      {data.groups.map((group) => <PhoneGroup key={group.accountId} group={group} openId={openId} prompts={data.changePrompts} onToggle={(id) => setOpenId((current) => (current === id ? null : id))} onOpenChat={onOpenChat} />)}
      {data.groups.length === 0 && <p className="finance-phone-empty">{filtered ? 'No hay movimientos con ese filtro.' : 'No hay movimientos en este mes.'}</p>}
    </>}
  </div>
}

interface GroupProps {
  group: MovementGroup
  openId: string | null
  prompts: Record<string, string>
  onToggle: (id: string) => void
  onOpenChat: (prompt: string) => void
}

function PhoneGroup({ group, openId, prompts, onToggle, onOpenChat }: GroupProps) {
  const total = group.totals.length > 0
    ? formatMoneyList(group.totals)
    : group.savings.length > 0 ? group.savings.map((money) => `+ ${formatMoney(money, 0)}`).join(' · ') : '—'
  return <section className="finance-phone-group" aria-label={group.name}>
    <div className="finance-phone-group__head">
      <span className="finance-phone-line__main">
        <span className="finance-phone-group__name">{group.name}</span>
        <span className="finance-phone-sub">{group.statement ? `Resumen ${formatMoney(group.statement.total)} · vence ${formatShortDate(group.statement.dueDate)}` : group.kindLabel}</span>
      </span>
      <strong className={`finance-phone-nowrap${group.totals.length === 0 && group.savings.length > 0 ? ' finance-teal' : ''}`}>{total}</strong>
    </div>
    {group.rows.map((row) => <PhoneMovementRow key={row.id} row={row} open={row.id === openId} prompt={prompts[row.id] ?? null} onToggle={() => onToggle(row.id)} onOpenChat={onOpenChat} />)}
  </section>
}

interface RowProps {
  row: MovementRow
  open: boolean
  prompt: string | null
  onToggle: () => void
  onOpenChat: (prompt: string) => void
}

function PhoneMovementRow({ row, open, prompt, onToggle, onOpenChat }: RowProps) {
  const entry = useFinanceEntry()
  const tone = movementTone(row)
  const category = row.exchange ? 'Ahorro' : row.categoryName ?? (row.uncategorized ? 'Sin categoría' : '—')
  return <div className="finance-phone-movement" data-open={open || undefined}>
    <button type="button" className="finance-phone-movement__row" aria-expanded={open} onClick={onToggle}>
      <span className="finance-phone-line__main">
        <span className={`finance-phone-ellipsis${tone === 'off' ? ' finance-tone--off' : ''}`}>{row.description || movementKindLabel(row.kind)}</span>
        <span className={row.flagged ? 'finance-phone-sub finance-warn-text' : 'finance-phone-sub'}>{formatShortDate(row.purchaseDate)} · {movementMeta(row)}</span>
      </span>
      <strong className={`finance-phone-nowrap finance-tone--${tone}`}>{movementAmount(row)}</strong>
    </button>
    {open && <div className="finance-phone-movement__detail">
      {row.note && <p className="finance-phone-note">{row.note}</p>}
      <dl className="finance-phone-facts">
        <dt>Compra</dt><dd>{formatLongDate(row.purchaseDate)}</dd>
        <dt>Cuenta en</dt><dd>{formatShortDate(row.effectiveDate)}{row.countsOnStatementDue && ' (vence el resumen)'}</dd>
        <dt>Categoría</dt><dd>{category}</dd>
        <dt>Servicio</dt><dd>{row.serviceName ?? '—'}</dd>
        <dt>Cargado desde</dt><dd>{row.origin}</dd>
        <dt>Estado</dt><dd>{movementStatusLabel(row.status)}</dd>
      </dl>
      {entry && <button type="button" className="finance-phone-button finance-phone-button--accent" onClick={() => entry.openEntry({ kind: 'movement', id: row.id })}>Editar</button>}
      {prompt && <button type="button" className="finance-phone-button" onClick={() => onOpenChat(prompt)}>Pedir un cambio en el chat</button>}
    </div>}
  </div>
}
