import { useCallback } from 'react'
import { ChevronRight, Plus } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getManage } from '../services/financeEntryService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { useFinanceEntry } from './entry/financeEntryContext'
import { FinanceStatus } from './FinanceParts'

/** «Administrar»: what is set up, to edit or delete it (nothing is «deactivated»). */
export function FinanceManageTab({ library }: { library: NotiaLibrary }) {
  const load = useCallback(() => getManage(library), [library])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo leer lo configurado.')
  const entry = useFinanceEntry()
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  return <div className="finance-manage">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    {data.map((section) => <section key={section.key} className="finance-card finance-card--list" aria-label={section.title}>
      <div className="finance-card__head">
        <h2>{section.title}</h2>
        <button type="button" className="finance-button finance-button--ghost finance-button--small" onClick={() => entry?.openEntry({ kind: section.addKind, prefill: section.key === 'income-categories' ? { kind: 'income' } : undefined })}>
          <Plus size={14} aria-hidden="true" />{section.addLabel}
        </button>
      </div>
      {section.rows.length === 0 ? <p className="finance-empty">{section.empty}</p> : <ul className="finance-manage__rows" role="list">
        {section.rows.map((row) => <li key={row.id}>
          <button type="button" className={`finance-manage__row${row.muted ? ' finance-manage__row--muted' : ''}`} onClick={() => entry?.openEntry({ kind: row.kind, id: row.id })}>
            <span className="finance-manage__main">
              <span className="finance-manage__title">{row.title}</span>
              <span className="finance-manage__sub">{row.subtitle}</span>
            </span>
            <ChevronRight size={16} aria-hidden="true" />
          </button>
        </li>)}
      </ul>}
    </section>)}
  </div>
}
