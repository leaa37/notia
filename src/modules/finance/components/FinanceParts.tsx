import type { ReactNode } from 'react'
import { formatExact, formatMoneyList, formatShortDate } from '../engines/financeFormat'
import type { Money, MovementRow } from '../types/financeScreen'
import { useFinanceEntry } from './entry/financeEntryContext'

/** Loading or failing, with a retry. */
export function FinanceStatus({ isLoading, error, onRetry }: { isLoading: boolean; error: string | null; onRetry: () => void }) {
  if (error) {
    return <div className="finance-alert" role="alert">
      <span>{error}</span>
      <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={onRetry}>Reintentar</button>
    </div>
  }
  return isLoading ? <p className="finance-status" role="status">Cargando…</p> : null
}

export function FinanceCard({ title, sub, action, className, children }: { title?: ReactNode; sub?: ReactNode; action?: ReactNode; className?: string; children: ReactNode }) {
  return <section className={`finance-card${className ? ` ${className}` : ''}`}>
    {(title || action) && <div className="finance-card__head">
      <div>
        {title && <h2>{title}</h2>}
        {sub && <p className="finance-card__sub">{sub}</p>}
      </div>
      {action}
    </div>}
    {children}
  </section>
}

/** A bar of `percent` (0 to 100 and beyond), colored by its tone. */
export function FinanceBar({ percent, tone = 'teal', label }: { percent: number; tone?: 'teal' | 'cards' | 'warn' | 'alert' | 'muted'; label?: string }) {
  const width = Math.max(0, Math.min(100, percent))
  return <div className="finance-bar" role={label ? 'img' : undefined} aria-label={label}>
    <div className={`finance-bar__fill finance-bar__fill--${tone}`} style={{ width: `${width}%` }} />
  </div>
}

export function FinancePill({ tone, children }: { tone: 'paid' | 'pending' | 'late' | 'neutral'; children: ReactNode }) {
  return <span className={`finance-pill finance-pill--${tone}`}>{children}</span>
}

export function MoneyText({ list, empty = '—', decimals = 0 }: { list: Money[]; empty?: string; decimals?: 0 | 2 }) {
  return <>{list.length === 0 ? empty : formatMoneyList(list, decimals)}</>
}

/** What is still to pay: the exact amounts, and what has none yet («2 servicios»). */
export function PendingText({ pending, note }: { pending: Money[]; note: string | null }) {
  if (pending.length === 0 && !note) return <>Nada</>
  return <>{[pending.length > 0 ? formatMoneyList(pending, 0) : null, note].filter(Boolean).join(' + ')}</>
}

/** What a movement is, without repeating its name. */
function movementMeta(row: MovementRow): string {
  if (row.detail) return row.detail
  if (row.category && row.category !== row.name) return row.category
  if (row.category) return row.kind === 'income' ? 'Ingreso' : 'Gasto'
  return 'Sin categoría'
}

/** Movements; touching one opens its form. */
export function MovementList({ rows, empty, showDate = true }: { rows: MovementRow[]; empty: string; showDate?: boolean }) {
  const entry = useFinanceEntry()
  if (rows.length === 0) return <p className="finance-empty">{empty}</p>
  return <ul className="finance-movements" role="list">
    {rows.map((row) => <li key={row.id}>
      <button type="button" className="finance-movement" onClick={() => entry?.openEntry({ kind: 'movement', id: row.id })} aria-label={`${row.name}, ${formatExact(row.amount, row.currency)}`}>
        {showDate && <span className="finance-movement__date">{formatShortDate(row.date)}</span>}
        <span className="finance-movement__main">
          <span className="finance-movement__name">{row.name}</span>
          <span className="finance-movement__meta">{[movementMeta(row), row.account].filter(Boolean).join(' · ')}</span>
        </span>
        <span className={`finance-movement__amount${row.kind === 'income' ? ' finance-movement__amount--income' : ''}`}>
          {row.kind === 'income' ? '+ ' : ''}{formatExact(row.amount, row.currency)}
        </span>
      </button>
    </li>)}
  </ul>
}
