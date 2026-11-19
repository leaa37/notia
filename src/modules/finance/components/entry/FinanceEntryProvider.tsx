import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { NotiaModalShell } from '../../../../components/notia/NotiaModalShell'
import type { NotiaLibrary } from '../../../../types/notia'
import { financeErrorMessage } from '../../engines/financeError'
import { notifyFinanceDataChanged } from '../../services/financeDataEvents'
import { applyEntry, getEntryForm, getEntryMenu, newRequestId, resolveReview } from '../../services/financeEntryService'
import type { EntryForm, EntryMenuGroup, EntryRequest, EntryValue, EntryValues } from '../../types/financeEntry'
import { FinanceExportDialog } from '../FinanceHistory'
import { FinanceFormFields } from './FinanceFormFields'
import { FinanceEntryContext, type FinanceEntryApi } from './financeEntryContext'

const NOTICE_MS = 5000

/**
 * Owns the dialogs of manual loads. They render beside the view (in
 * `.finance-layer`), not inside its size container, so they stay fixed.
 */
export function FinanceEntryProvider({ library, phone, children }: { library: NotiaLibrary; phone: boolean; children: ReactNode }) {
  const [request, setRequest] = useState<EntryRequest | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const [exportMonth, setExportMonth] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const noticeTimer = useRef<number | null>(null)

  const announce = useCallback((message: string) => {
    setNotice(message)
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current)
    noticeTimer.current = window.setTimeout(() => setNotice(null), NOTICE_MS)
  }, [])
  useEffect(() => () => {
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current)
  }, [])

  const done = useCallback((message: string) => {
    setRequest(null)
    announce(message)
    notifyFinanceDataChanged()
  }, [announce])

  const api = useMemo<FinanceEntryApi>(() => ({
    openEntry: (next) => {
      setMenuOpen(false)
      setRequest(next)
    },
    openMenu: () => setMenuOpen(true),
    openExport: (month) => setExportMonth(month),
    resolve: async (reviewItemId, optionId) => {
      try {
        const result = await resolveReview(library, reviewItemId, optionId)
        announce(result.message)
        if (result.ok) notifyFinanceDataChanged()
      } catch (reason) {
        announce(financeErrorMessage(reason, 'No se pudo resolver el caso.'))
      }
    },
  }), [announce, library])

  return <FinanceEntryContext.Provider value={api}>
    {children}
    <div className="finance-layer">
      {menuOpen && <EntryMenu library={library} phone={phone} onPick={(kind) => api.openEntry({ kind })} onClose={() => setMenuOpen(false)} />}
      {exportMonth && <FinanceExportDialog library={library} month={exportMonth} phone={phone} onClose={() => setExportMonth(null)} />}
      {request && <EntryDialog key={`${request.kind}:${request.id ?? 'new'}`} library={library} request={request} phone={phone} onDone={done} onClose={() => setRequest(null)} />}
      <div className="finance-notice" role="status" aria-live="polite">{notice && <span className="finance-notice__text">{notice}</span>}</div>
    </div>
  </FinanceEntryContext.Provider>
}

function Header({ title, onClose }: { title: string; onClose: () => void }) {
  return <header className="finance-dialog__head">
    <h2 className="finance-dialog__title">{title}</h2>
    <button type="button" className="finance-icon-button" aria-label="Cerrar" onClick={onClose}><X size={18} aria-hidden="true" /></button>
  </header>
}

function EntryMenu({ library, phone, onPick, onClose }: { library: NotiaLibrary; phone: boolean; onPick: (kind: string) => void; onClose: () => void }) {
  const [groups, setGroups] = useState<EntryMenuGroup[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let active = true
    getEntryMenu(library)
      .then((value) => { if (active) setGroups(value) })
      .catch((reason) => { if (active) setError(financeErrorMessage(reason, 'No se pudo abrir el menú de cargas.')) })
    return () => { active = false }
  }, [library])
  return <NotiaModalShell open onClose={onClose} size="md" fill={phone} panelClassName="finance-dialog">
    <div className="finance-dialog__body" role="dialog" aria-modal="true" aria-label="Cargar en Finanzas">
      <Header title="Cargar" onClose={onClose} />
      {error && <p className="finance-form__error" role="alert">{error}</p>}
      {!groups && !error && <p className="finance-card__sub">Cargando…</p>}
      <div className="finance-menu">
        {groups?.map((group) => <section key={group.title} className="finance-menu__group" aria-label={group.title}>
          <h3 className="finance-mono-label">{group.title}</h3>
          <div className="finance-menu__items">
            {group.items.map((item) => <button key={item.kind} type="button" className="finance-menu__item" onClick={() => onPick(item.kind)}>
              <span className="finance-menu__label">{item.label}</span>
              <span className="finance-card__sub">{item.description}</span>
            </button>)}
          </div>
        </section>)}
      </div>
    </div>
  </NotiaModalShell>
}

function initialValues(form: EntryForm): EntryValues {
  return Object.fromEntries(form.fields.map((field) => [field.key, field.value]))
}

type Pending = { kind: 'delete' } | { kind: 'action'; id: string; label: string; confirm: string | null }

function EntryDialog({ library, request, phone, onDone, onClose }: { library: NotiaLibrary; request: EntryRequest; phone: boolean; onDone: (message: string) => void; onClose: () => void }) {
  const [form, setForm] = useState<EntryForm | null>(null)
  const [values, setValues] = useState<EntryValues>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [message, setMessage] = useState<string | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<Pending | null>(null)
  // One id per attempt of the same load: retrying after a lost answer is
  // not applied twice.
  const requestId = useRef(newRequestId())

  const load = useCallback(() => {
    setLoadError(null)
    getEntryForm(library, request)
      .then((value) => {
        setForm(value)
        setValues(initialValues(value))
      })
      .catch((reason) => setLoadError(financeErrorMessage(reason, 'No se pudo abrir el formulario.')))
  }, [library, request])
  useEffect(load, [load])

  const change = (key: string, value: EntryValue) => {
    setValues((current) => ({ ...current, [key]: value }))
    setErrors((current) => {
      if (!(key in current)) return current
      const next = { ...current }
      delete next[key]
      return next
    })
  }

  const run = async (action: string, sent: EntryValues | undefined) => {
    if (!form) return
    setBusy(true)
    setMessage(null)
    try {
      const result = await applyEntry(library, { kind: form.kind, id: form.id, action, values: sent, requestId: requestId.current })
      if (result.ok) {
        onDone(result.duplicate ? `${result.message} (ya estaba cargado)` : result.message)
        return
      }
      setErrors(result.errors)
      setMessage(result.message)
      // What failed was not applied: the next attempt is a new one.
      requestId.current = newRequestId()
    } catch (reason) {
      setMessage(financeErrorMessage(reason, 'No se pudo guardar. Probá de nuevo.'))
    } finally {
      setBusy(false)
      setPending(null)
    }
  }

  const title = form?.title ?? 'Cargar'
  return <NotiaModalShell open onClose={busy ? () => undefined : onClose} size="lg" fill={phone} panelClassName={`finance-dialog${phone ? ' finance-dialog--phone' : ''}`}>
    <form
      className="finance-dialog__body finance-form"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      aria-busy={busy}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void run('save', values)
      }}
    >
      <Header title={title} onClose={onClose} />
      {loadError && <div className="finance-form__problem" role="alert">
        <p>{loadError}</p>
        <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={load}>Reintentar</button>
      </div>}
      {!form && !loadError && <p className="finance-card__sub">Cargando el formulario…</p>}
      {form && <>
        {form.intro && <p className="finance-form__intro">{form.intro}</p>}
        <FinanceFormFields fields={form.fields} values={values} errors={errors} disabled={busy} onChange={change} idPrefix={`finance-${form.kind}`} />
        {/* A field's own error is already beside it. */}
        {message && !Object.values(errors).includes(message) && <p className="finance-form__message" role="alert">{message}</p>}
        {pending && <div className="finance-form__confirm" role="alertdialog" aria-label="Confirmar">
          <p>{pending.kind === 'delete' ? form.delete?.confirm : pending.confirm}</p>
          <div className="finance-form__actions">
            <button type="button" className="finance-button finance-button--ghost" disabled={busy} onClick={() => setPending(null)}>No</button>
            <button type="button" className="finance-button finance-button--danger" disabled={busy} onClick={() => void run(pending.kind === 'delete' ? 'delete' : pending.id, undefined)}>
              {busy ? 'Un momento…' : pending.kind === 'delete' ? form.delete?.label : pending.label}
            </button>
          </div>
        </div>}
        <footer className="finance-form__footer">
          <div className="finance-form__secondary">
            {form.delete && <button type="button" className="finance-button finance-button--quiet-danger" disabled={busy} onClick={() => setPending({ kind: 'delete' })}>{form.delete.label}</button>}
            {form.actions.map((action) => <button key={action.id} type="button" className="finance-button finance-button--quiet-outline" disabled={busy} onClick={() => {
              if (action.confirm) setPending({ kind: 'action', ...action })
              else void run(action.id, undefined)
            }}>{action.label}</button>)}
          </div>
          <div className="finance-form__actions">
            <button type="button" className="finance-button finance-button--ghost" disabled={busy} onClick={onClose}>Cancelar</button>
            <button type="submit" className="finance-button finance-button--primary" disabled={busy}>{busy ? 'Guardando…' : form.submit}</button>
          </div>
        </footer>
      </>}
    </form>
  </NotiaModalShell>
}
