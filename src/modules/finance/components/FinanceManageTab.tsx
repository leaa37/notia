import { useCallback } from 'react'
import { ChevronRight, Plus } from 'lucide-react'
import type { NotiaLibrary } from '../../../types/notia'
import { getManage } from '../services/financeEntryService'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { FinanceStatus } from './FinanceStatus'
import { useFinanceEntry } from './entry/financeEntryContext'

/**
 * «Administrar»: accounts, categories, services, reserves, loans, budgets,
 * plans of installments and merchants, each with its form. Rust lists them
 * and decides what each row says.
 */
export function FinanceManageTab({ library }: { library: NotiaLibrary }) {
  const entry = useFinanceEntry()
  const load = useCallback(() => getManage(library), [library])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo cargar lo que se administra.')
  if (!data) return <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} />
  return <div className="finance-manage">
    {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
    {data.map((section) => <section key={section.key} className="finance-card finance-manage__section" aria-labelledby={`finance-manage-${section.key}`}>
      <div className="finance-card__head finance-card__head--baseline">
        <h2 id={`finance-manage-${section.key}`}>{section.title}<span className="finance-muted"> · {section.rows.length}</span></h2>
        {section.addKind && <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={() => entry?.openEntry({ kind: section.addKind ?? '' })}>
          <Plus size={14} aria-hidden="true" />{section.addLabel}
        </button>}
      </div>
      {section.rows.length === 0 && <p className="finance-empty">{section.empty}</p>}
      <div className="finance-list">
        {section.rows.map((row) => <button key={row.id} type="button" className={`finance-manage__row${row.inactive ? ' finance-manage__row--inactive' : ''}`} onClick={() => entry?.openEntry({ kind: row.kind, id: row.id })}>
          <span className="finance-manage__main">
            <span className="finance-manage__title">{row.title}{row.inactive && <span className="finance-pill">Inactivo</span>}</span>
            <span className="finance-card__sub">{row.subtitle}</span>
          </span>
          <ChevronRight size={18} aria-hidden="true" className="finance-muted" />
        </button>)}
      </div>
    </section>)}
  </div>
}
