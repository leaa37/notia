import { useCallback, useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { NotiaModalShell } from '../../../components/notia/NotiaModalShell'
import type { NotiaLibrary } from '../../../types/notia'
import { financeErrorMessage } from '../engines/financeError'
import { formatMonthTitle } from '../engines/financeFormat'
import { exportFinance } from '../services/financeEntryService'
import { searchFinanceHistory } from '../services/financeService'
import type { FinanceSearch, MovementFilter } from '../types/financeScreen'
import { FinanceStatus, MoneyText, MovementList } from './FinanceParts'

/** Every month at once, with the filters of «Movimientos», 200 at a time. */
export function FinanceHistory({ library, filter }: { library: NotiaLibrary; filter: MovementFilter }) {
  const [pages, setPages] = useState<FinanceSearch[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const load = useCallback((page: number) => {
    setLoading(true)
    setError(null)
    searchFinanceHistory(library, filter, page)
      .then((result) => setPages((current) => (page === 0 ? [result] : [...current, result])))
      .catch((reason) => setError(financeErrorMessage(reason, 'No se pudo buscar en el historial.')))
      .finally(() => setLoading(false))
  }, [filter, library])
  useEffect(() => load(0), [load])

  const first = pages[0]
  if (!first) return <FinanceStatus isLoading={loading} error={error} onRetry={() => load(0)} />
  const last = pages[pages.length - 1]
  const months = pages.flatMap((page) => page.months)
  return <div className="finance-history">
    <div className="finance-totals">
      <span>Gastos <strong><MoneyText list={first.expenses} empty="$ 0" decimals={2} /></strong></span>
      <span>Ingresos <strong className="finance-teal"><MoneyText list={first.income} empty="$ 0" decimals={2} /></strong></span>
      <span className="finance-muted">{first.count} {first.count === 1 ? 'movimiento' : 'movimientos'} en todo el historial</span>
    </div>
    {months.length === 0 && <p className="finance-empty finance-empty--boxed">Nada coincide.</p>}
    {months.map((month, index) => <section key={`${month.month}:${index}`} className="finance-card finance-card--list">
      <h3 className="finance-mono-label">{formatMonthTitle(month.month)}</h3>
      <MovementList rows={month.rows} empty="" />
    </section>)}
    {error && <FinanceStatus isLoading={false} error={error} onRetry={() => load(last.page + 1)} />}
    {last.hasMore && <button type="button" className="finance-button finance-button--ghost" disabled={loading} onClick={() => load(last.page + 1)}>{loading ? 'Cargando…' : 'Ver más'}</button>}
  </div>
}

type Range = 'month' | 'year' | 'all'

function rangeOf(range: Range, month: string): { from: string | null; to: string | null } {
  if (range === 'all') return { from: null, to: null }
  if (range === 'year') return { from: `${month.slice(0, 4)}-01-01`, to: `${month.slice(0, 4)}-12-31` }
  const [year, number] = month.split('-').map(Number)
  const last = new Date(year, number, 0).getDate()
  return { from: `${month}-01`, to: `${month}-${String(last).padStart(2, '0')}` }
}

/** Saves the movements as Excel or CSV, built by Rust. */
export function FinanceExportDialog({ library, month, phone, onClose }: { library: NotiaLibrary; month: string; phone: boolean; onClose: () => void }) {
  const [range, setRange] = useState<Range>('month')
  const [format, setFormat] = useState<'xlsx' | 'csv'>('xlsx')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  const save = async () => {
    setBusy(true)
    setMessage(null)
    try {
      const file = await exportFinance(library, { ...rangeOf(range, month), format })
      const bytes = Uint8Array.from(atob(file.base64), (character) => character.charCodeAt(0))
      const url = URL.createObjectURL(new Blob([bytes], { type: file.mediaType }))
      const link = document.createElement('a')
      link.href = url
      link.download = file.fileName
      document.body.appendChild(link)
      link.click()
      link.remove()
      window.setTimeout(() => URL.revokeObjectURL(url), 1000)
      setMessage(`Listo: ${file.rows} ${file.rows === 1 ? 'movimiento' : 'movimientos'} en ${file.fileName}.`)
    } catch (reason) {
      setMessage(financeErrorMessage(reason, 'No se pudo exportar.'))
    } finally {
      setBusy(false)
    }
  }

  return <NotiaModalShell open onClose={busy ? () => undefined : onClose} size="sm" fill={phone} panelClassName="finance-dialog">
    <div className="finance-dialog__body finance-form" role="dialog" aria-modal="true" aria-label="Exportar movimientos">
      <header className="finance-dialog__head">
        <h2 className="finance-dialog__title">Exportar movimientos</h2>
        <button type="button" className="finance-icon-button" aria-label="Cerrar" onClick={onClose}><X size={18} aria-hidden="true" /></button>
      </header>
      <fieldset className="finance-form__field">
        <legend className="finance-form__label">Qué</legend>
        <div className="finance-segmented" role="radiogroup" aria-label="Qué">
          {([['month', formatMonthTitle(month)], ['year', `Todo ${month.slice(0, 4)}`], ['all', 'Todo']] as const).map(([value, label]) => <button key={value} type="button" role="radio" aria-checked={range === value} onClick={() => setRange(value)}>{label}</button>)}
        </div>
      </fieldset>
      <fieldset className="finance-form__field">
        <legend className="finance-form__label">Formato</legend>
        <div className="finance-segmented" role="radiogroup" aria-label="Formato">
          {([['xlsx', 'Excel'], ['csv', 'CSV']] as const).map(([value, label]) => <button key={value} type="button" role="radio" aria-checked={format === value} onClick={() => setFormat(value)}>{label}</button>)}
        </div>
      </fieldset>
      {message && <p className="finance-form__message" role="status">{message}</p>}
      <footer className="finance-form__footer">
        <div className="finance-form__actions">
          <button type="button" className="finance-button finance-button--ghost" disabled={busy} onClick={onClose}>Cerrar</button>
          <button type="button" className="finance-button finance-button--primary" disabled={busy} onClick={() => void save()}>{busy ? 'Armando…' : 'Descargar'}</button>
        </div>
      </footer>
    </div>
  </NotiaModalShell>
}
