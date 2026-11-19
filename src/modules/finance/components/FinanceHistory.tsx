import { useCallback, useEffect, useState } from 'react'
import { Download, Search } from 'lucide-react'
import { NotiaModalShell } from '../../../components/notia/NotiaModalShell'
import type { NotiaLibrary } from '../../../types/notia'
import { financeErrorMessage } from '../engines/financeError'
import { formatMoneyList, formatMonthTitle, formatShortDate, plural } from '../engines/financeFormat'
import { movementAmount, movementKindLabel, movementMeta, movementTone } from '../engines/financeMovementText'
import { useFinanceResource } from '../hooks/useFinanceResource'
import { exportFinance, searchHistory } from '../services/financeEntryService'
import type { FinanceSearch } from '../types/financeEntry'
import { FinanceStatus } from './FinanceStatus'
import { useFinanceEntry } from './entry/financeEntryContext'

const SEARCH_DELAY_MS = 300

/**
 * Every month at once: what the search finds in the whole history, the most
 * recent month first. Rust searches, groups and adds up; a row opens its
 * form to edit it.
 */
export function FinanceHistory({ library, phone }: { library: NotiaLibrary; phone: boolean }) {
  const entry = useFinanceEntry()
  const [search, setSearch] = useState('')
  const [applied, setApplied] = useState('')
  const [pages, setPages] = useState(1)
  const [more, setMore] = useState<FinanceSearch[]>([])
  const [loadingMore, setLoadingMore] = useState(false)
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setApplied(search)
      setPages(1)
      setMore([])
    }, SEARCH_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [search])
  const load = useCallback(() => searchHistory(library, { search: applied, filter: 'all', page: 0 }), [applied, library])
  const { data, error, isLoading, reload } = useFinanceResource(load, 'No se pudo buscar en el historial.')
  const pagesShown = [data, ...more].filter((page): page is FinanceSearch => page !== null)
  const last = pagesShown[pagesShown.length - 1]

  const loadMore = () => {
    setLoadingMore(true)
    searchHistory(library, { search: applied, filter: 'all', page: pages })
      .then((page) => {
        setMore((current) => [...current, page])
        setPages((current) => current + 1)
      })
      .catch(() => undefined)
      .finally(() => setLoadingMore(false))
  }

  const months = pagesShown.flatMap((page) => page.months)
  return <section className={phone ? 'finance-phone finance-history-view' : 'finance-history-view'} aria-label="Todo el historial">
    <div className="finance-filters">
      <label className={phone ? 'finance-phone-search' : 'finance-search'}>
        <Search size={16} aria-hidden="true" />
        <span className="finance-visually-hidden">Buscar en todo el historial</span>
        <input type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Buscar en todos los meses: Netflix, Coto…" />
      </label>
    </div>
    {!data ? <FinanceStatus isLoading={isLoading} error={error} onRetry={reload} /> : <>
      {error && <FinanceStatus isLoading={false} error={error} onRetry={reload} />}
      <p className="finance-card__sub" aria-live="polite">
        {plural(data.count, 'movimiento')}{data.expenseTotals.length > 0 && ` · gastos ${formatMoneyList(data.expenseTotals)}`}
      </p>
      {months.length === 0 && <p className="finance-empty finance-empty--boxed">{applied.trim() ? 'No hay movimientos con esa búsqueda.' : 'Todavía no hay movimientos.'}</p>}
      {months.map((month, index) => <div key={`${month.month}-${index}`} className="finance-group">
        <div className="finance-group__head">
          <span className="finance-group__title"><span className="finance-group__name">{formatMonthTitle(month.month)}</span></span>
          <strong>{formatMoneyList(month.expenseTotals)}</strong>
        </div>
        {month.rows.map((row) => {
          const tone = movementTone(row)
          return <button key={row.id} type="button" className="finance-row" onClick={() => entry?.openEntry({ kind: 'movement', id: row.id })}>
            <span className="finance-row__date">{formatShortDate(row.purchaseDate)}</span>
            <span className="finance-row__main">
              <span className={`finance-row__desc${tone === 'off' ? ' finance-tone--off' : ''}`}>{row.description || movementKindLabel(row.kind)}</span>
              <span className="finance-row__meta">{row.accountName} · {movementMeta(row)}</span>
            </span>
            <span className={`finance-row__amount finance-tone--${tone}`}>{movementAmount(row)}</span>
          </button>
        })}
      </div>)}
      {last?.hasMore && <button type="button" className="finance-button finance-button--ghost" disabled={loadingMore} onClick={loadMore}>{loadingMore ? 'Cargando…' : 'Ver más'}</button>}
    </>}
  </section>
}

type Range = 'month' | 'year' | 'all'

function rangeOf(range: Range, month: string): { from: string | null; to: string | null } {
  if (range === 'all') return { from: null, to: null }
  const [year, monthNumber] = month.split('-').map(Number)
  if (range === 'year') return { from: `${year}-01-01`, to: `${year}-12-31` }
  const last = new Date(year, monthNumber, 0).getDate()
  return { from: `${month}-01`, to: `${month}-${String(last).padStart(2, '0')}` }
}

function download(base64: string, mediaType: string, name: string) {
  const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0))
  const url = URL.createObjectURL(new Blob([bytes], { type: mediaType }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = name
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000)
}

/** Exports the movements of the month, the year or the whole history (in the dialog layer). */
export function FinanceExportDialog({ library, month, phone, onClose }: { library: NotiaLibrary; month: string; phone: boolean; onClose: () => void }) {
  const [range, setRange] = useState<Range>('month')
  const [format, setFormat] = useState<'xlsx' | 'csv'>('xlsx')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const year = month.slice(0, 4)
  const run = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const file = await exportFinance(library, { ...rangeOf(range, month), format })
      download(file.base64, file.mediaType, file.fileName)
      setMessage(`Listo: ${plural(file.rows, 'movimiento')} en ${file.fileName}.`)
    } catch (reason) {
      setMessage(financeErrorMessage(reason, 'No se pudo exportar.'))
    } finally {
      setBusy(false)
    }
  }
  return <NotiaModalShell open onClose={onClose} size="sm" fill={phone} panelClassName="finance-dialog">
      <div className="finance-dialog__body finance-form" role="dialog" aria-modal="true" aria-label="Exportar movimientos">
        <header className="finance-dialog__head"><h2 className="finance-dialog__title">Exportar movimientos</h2></header>
        <fieldset className="finance-form__choices">
          <legend className="finance-form__label">Qué</legend>
          {([['month', `${formatMonthTitle(month)}`], ['year', `Todo ${year}`], ['all', 'Todo el historial']] as Array<[Range, string]>).map(([value, label]) => <label key={value} className="finance-form__choice">
            <input type="radio" name="finance-export-range" value={value} checked={range === value} onChange={() => setRange(value)} />{label}
          </label>)}
        </fieldset>
        <fieldset className="finance-form__choices">
          <legend className="finance-form__label">Formato</legend>
          <label className="finance-form__choice"><input type="radio" name="finance-export-format" value="xlsx" checked={format === 'xlsx'} onChange={() => setFormat('xlsx')} />Excel (.xlsx)</label>
          <label className="finance-form__choice"><input type="radio" name="finance-export-format" value="csv" checked={format === 'csv'} onChange={() => setFormat('csv')} />CSV (.csv)</label>
        </fieldset>
        {message && <p className="finance-form__message" role="status">{message}</p>}
        <footer className="finance-form__footer">
          <span />
          <div className="finance-form__actions">
            <button type="button" className="finance-button finance-button--ghost" onClick={onClose}>Cerrar</button>
            <button type="button" className="finance-button finance-button--primary" disabled={busy} onClick={() => void run()}>
              <Download size={16} aria-hidden="true" />{busy ? 'Exportando…' : 'Exportar'}
            </button>
          </div>
        </footer>
      </div>
    </NotiaModalShell>
}
