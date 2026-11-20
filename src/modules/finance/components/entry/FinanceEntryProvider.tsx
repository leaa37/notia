import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { NotiaModalShell } from '../../../../components/notia/NotiaModalShell'
import type { NotiaLibrary } from '../../../../types/notia'
import { financeErrorMessage } from '../../engines/financeError'
import { notifyFinanceDataChanged } from '../../services/financeDataEvents'
import { applyEntry, getEntryForm, getEntryMenu, newRequestId } from '../../services/financeEntryService'
import type { EntryField, EntryForm, EntryMenuGroup, EntryRequest, EntryValue, EntryValues, Undo } from '../../types/financeEntry'
import { FinanceExportDialog } from '../FinanceHistory'
import { FinanceFormFields } from './FinanceFormFields'
import { initialValues } from './entryValues'
import { FinanceEntryContext, type FinanceEntryApi } from './financeEntryContext'

const NOTICE_MS = 6000

interface Notice {
  message: string
  undo: Undo | null
  error?: boolean
}

/**
 * Owns the dialogs of manual loads. They render beside the view (in
 * `.finance-layer`), not inside its size container, so they stay fixed.
 */
export function FinanceEntryProvider({ library, phone, children }: { library: NotiaLibrary; phone: boolean; children: ReactNode }) {
  const [request, setRequest] = useState<EntryRequest | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const [exportMonth, setExportMonth] = useState<string | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const noticeTimer = useRef<number | null>(null)

  const announce = useCallback((next: Notice) => {
    setNotice(next)
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current)
    noticeTimer.current = window.setTimeout(() => setNotice(null), NOTICE_MS)
  }, [])
  useEffect(() => () => {
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current)
  }, [])

  const done = useCallback((message: string, undo: Undo | null) => {
    setRequest(null)
    announce({ message, undo })
    notifyFinanceDataChanged()
  }, [announce])

  const undo = useCallback(async (what: Undo) => {
    setNotice(null)
    try {
      const result = await applyEntry(library, { kind: what.kind, id: what.id, action: what.action, values: what.values, requestId: newRequestId() })
      announce({ message: result.ok ? 'Listo, lo deshice.' : result.message, undo: null, error: !result.ok })
      notifyFinanceDataChanged()
    } catch (reason) {
      announce({ message: financeErrorMessage(reason, 'No se pudo deshacer.'), undo: null, error: true })
    }
  }, [announce, library])

  const api = useMemo<FinanceEntryApi>(() => ({
    openEntry: (next) => {
      setMenuOpen(false)
      setRequest(next)
    },
    openMenu: () => setMenuOpen(true),
    openExport: (month) => setExportMonth(month),
    quickApply: async (kind, values) => {
      try {
        const result = await applyEntry(library, { kind, id: null, values, requestId: newRequestId() })
        announce({ message: result.message, undo: result.ok ? result.undo : null, error: !result.ok })
        if (result.ok) notifyFinanceDataChanged()
        return result.ok
      } catch (reason) {
        announce({ message: financeErrorMessage(reason, 'No se pudo guardar.'), undo: null, error: true })
        return false
      }
    },
  }), [announce, library])

  return <FinanceEntryContext.Provider value={api}>
    {children}
    <div className="finance-layer">
      {menuOpen && <EntryMenu library={library} phone={phone} onPick={(kind) => api.openEntry({ kind })} onClose={() => setMenuOpen(false)} />}
      {exportMonth && <FinanceExportDialog library={library} month={exportMonth} phone={phone} onClose={() => setExportMonth(null)} />}
      {request && <EntryDialog key={`${request.kind}:${request.id ?? 'new'}`} library={library} request={request} phone={phone} onDone={done} onClose={() => setRequest(null)} />}
      <div className="finance-notice" role="status" aria-live="polite">
        {notice && <div className={`finance-notice__box${notice.error ? ' finance-notice__box--error' : ''}`}>
          <span className="finance-notice__text">{notice.message}</span>
          {notice.undo && <button type="button" className="finance-notice__undo" onClick={() => void undo(notice.undo as Undo)}>Deshacer</button>}
        </div>}
      </div>
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
      {!groups && !error && <p className="finance-muted">Cargando…</p>}
      <div className="finance-menu">
        {groups?.map((group) => <section key={group.title} className="finance-menu__group" aria-label={group.title}>
          <h3 className="finance-mono-label">{group.title}</h3>
          <div className="finance-menu__items">
            {group.items.map((item) => <button key={item.kind} type="button" className="finance-menu__item" onClick={() => onPick(item.kind)}>
              <span className="finance-menu__label">{item.label}</span>
              <span className="finance-menu__description">{item.description}</span>
            </button>)}
          </div>
        </section>)}
      </div>
    </div>
  </NotiaModalShell>
}

function same(a: EntryValue | undefined, b: EntryValue | undefined): boolean {
  return JSON.stringify(a ?? '') === JSON.stringify(b ?? '')
}

type Pending = 'delete' | 'discard' | null

function EntryDialog({ library, request, phone, onDone, onClose }: { library: NotiaLibrary; request: EntryRequest; phone: boolean; onDone: (message: string, undo: Undo | null) => void; onClose: () => void }) {
  const [form, setForm] = useState<EntryForm | null>(null)
  const [values, setValues] = useState<EntryValues>({})
  const [start, setStart] = useState<EntryValues>({})
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [message, setMessage] = useState<string | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState<Pending>(null)
  // One id per attempt of the same load: retrying after a lost answer is
  // not applied twice.
  const requestId = useRef(newRequestId())
  const reloads = useRef(0)

  const show = useCallback((next: EntryForm, keepStart: boolean) => {
    const nextValues = initialValues(next.fields)
    setForm(next)
    setValues(nextValues)
    if (!keepStart) setStart(nextValues)
  }, [])

  const load = useCallback(() => {
    setLoadError(null)
    getEntryForm(library, request)
      .then((value) => show(value, false))
      .catch((reason) => setLoadError(financeErrorMessage(reason, 'No se pudo abrir el formulario.')))
  }, [library, request, show])
  useEffect(load, [load])

  // A field with `reload` asks Rust for the form again, with what was typed.
  const reload = (next: EntryValues, changed: string) => {
    const ticket = ++reloads.current
    getEntryForm(library, { kind: form?.kind ?? request.kind, id: form ? form.id : request.id ?? null, prefill: { ...next, _changed: changed } })
      .then((value) => { if (ticket === reloads.current) show(value, true) })
      .catch((reason) => setMessage(financeErrorMessage(reason, 'No se pudo actualizar el formulario.')))
  }

  const change = (key: string, value: EntryValue, field: EntryField) => {
    const next = { ...values, [key]: value }
    setValues(next)
    setErrors((current) => {
      if (!(key in current)) return current
      const rest = { ...current }
      delete rest[key]
      return rest
    })
    if (field.reload && key === field.key) reload(next, key)
  }

  const dirty = form !== null && Object.keys(values).some((key) => !same(values[key], start[key]))
  // Nothing typed is lost by a click outside or Escape: it asks first.
  const requestClose = () => {
    if (busy) return
    if (dirty) setPending('discard')
    else onClose()
  }

  const run = async (action: 'save' | 'delete') => {
    if (!form) return
    setBusy(true)
    setMessage(null)
    try {
      const result = await applyEntry(library, { kind: form.kind, id: form.id, action, values: action === 'save' ? values : undefined, requestId: requestId.current })
      if (result.ok) {
        onDone(result.duplicate ? `${result.message} (ya estaba cargado)` : result.message, result.undo)
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
  return <NotiaModalShell open onClose={requestClose} size="md" fill={phone} panelClassName={`finance-dialog${phone ? ' finance-dialog--phone' : ''}`}>
    <form
      className="finance-dialog__body finance-form"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      aria-busy={busy}
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void run('save')
      }}
    >
      <Header title={title} onClose={requestClose} />
      {loadError && <div className="finance-form__problem" role="alert">
        <p>{loadError}</p>
        <button type="button" className="finance-button finance-button--outline finance-button--small" onClick={load}>Reintentar</button>
      </div>}
      {!form && !loadError && <p className="finance-muted">Cargando el formulario…</p>}
      {form && <>
        {form.intro && <p className="finance-form__intro">{form.intro}</p>}
        <FinanceFormFields fields={form.fields} values={values} errors={errors} disabled={busy} onChange={change} idPrefix={`finance-${form.kind}`} />
        {/* A field's own error is already beside it. */}
        {message && !Object.values(errors).includes(message) && <p className="finance-form__message" role="alert">{message}</p>}
        {pending && <div className="finance-form__confirm" role="alertdialog" aria-label="Confirmar">
          <p>{pending === 'delete' ? form.delete?.confirm : '¿Descartar lo que cargaste?'}</p>
          <div className="finance-form__actions">
            <button type="button" className="finance-button finance-button--ghost" disabled={busy} onClick={() => setPending(null)}>{pending === 'delete' ? 'No' : 'Seguir cargando'}</button>
            <button type="button" className="finance-button finance-button--danger" disabled={busy} onClick={() => (pending === 'delete' ? void run('delete') : onClose())}>
              {busy ? 'Un momento…' : pending === 'delete' ? form.delete?.label : 'Descartar'}
            </button>
          </div>
        </div>}
        <footer className="finance-form__footer">
          {form.delete && <button type="button" className="finance-button finance-button--quiet-danger" disabled={busy} onClick={() => setPending('delete')}>{form.delete.label}</button>}
          <div className="finance-form__actions">
            <button type="button" className="finance-button finance-button--ghost" disabled={busy} onClick={requestClose}>Cancelar</button>
            <button type="submit" className="finance-button finance-button--primary" disabled={busy}>{busy ? 'Guardando…' : form.submit}</button>
          </div>
        </footer>
      </>}
    </form>
  </NotiaModalShell>
}
