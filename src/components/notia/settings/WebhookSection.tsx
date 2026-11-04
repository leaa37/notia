import { useEffect, useState } from 'react'
import { Copy, Plus, Webhook } from 'lucide-react'
import { NotiaButton } from '../../common/NotiaButton'
import { SettingsBadge, SettingsCard, SettingsFooter, SettingsNotice, SettingsRow, SettingsSwitch, type SettingsTone } from './SettingsControls'
import {
  deleteWebhookAction,
  readWebhook,
  renewWebhookToken,
  saveWebhookAction,
  saveWebhookServer,
  type WebhookAction,
  type WebhookActionDraft,
  type WebhookCallStatus,
  type WebhookSaved,
  type WebhookView,
} from '../../../services/webhook/webhookRuntime'

/** How often the view is read again, for the state and the recent calls. */
const REFRESH_MS = 5000

const STATE_LABEL: Record<WebhookView['state'], string> = {
  off: 'Apagado',
  starting: 'Iniciando',
  listening: 'Escuchando',
  error: 'Con error',
}

const CALL_LABEL: Record<WebhookCallStatus, string> = {
  queued: 'En cola',
  running: 'En curso',
  sent: 'Enviado',
  silent: 'Sin mensaje',
  failed: 'Falló',
  rejected: 'Rechazada',
}

const timeFormat = new Intl.DateTimeFormat('es-AR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' })

function messageOf(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback
}

interface ServerDraft {
  enabled: boolean
  port: string
  network: boolean
}

function serverDraftOf(view: WebhookView): ServerDraft {
  return { enabled: view.enabled, port: String(view.port), network: view.network }
}

function sameServer(draft: ServerDraft, view: WebhookView): boolean {
  return draft.enabled === view.enabled && draft.port === String(view.port) && draft.network === view.network
}

interface WebhookSectionProps {
  /** Active library; the endpoint and its actions belong to it. */
  libraryId: string | null
}

/**
 * «Webhook»: the endpoint other programs call with `{ action, prompt }`, the
 * actions they may name (each with its own token) and the recent calls.
 * Rust listens, checks every call and decides everything.
 */
export function WebhookSection({ libraryId }: WebhookSectionProps) {
  const [view, setView] = useState<WebhookView | null>(null)
  /** What the person changed in the endpoint's form; `null` follows the stored settings. */
  const [serverEdits, setServerEdits] = useState<ServerDraft | null>(null)
  const [draft, setDraft] = useState<WebhookActionDraft | null>(null)
  const [token, setToken] = useState<{ action: string; value: string } | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [status, setStatus] = useState<{ tone: SettingsTone; message: string } | null>(null)

  useEffect(() => {
    if (!libraryId) return
    let active = true
    let timer: number | undefined
    const load = async () => {
      try {
        const next = await readWebhook(libraryId)
        if (active) setView(next)
      } catch (error) {
        if (active) setStatus({ tone: 'error', message: messageOf(error, 'No se pudo leer el webhook.') })
      }
      if (active) timer = window.setTimeout(() => { void load() }, REFRESH_MS)
    }
    void load()
    return () => {
      active = false
      window.clearTimeout(timer)
    }
  }, [libraryId])

  if (!libraryId) {
    return <SettingsNotice tone="idle">Abrí una biblioteca para configurar el webhook.</SettingsNotice>
  }
  if (view && !view.available) {
    return <SettingsNotice tone="idle">{view.unavailableReason}</SettingsNotice>
  }

  const run = async <T,>(key: string, work: () => Promise<T>, done: (result: T) => void, fallback: string) => {
    setBusy(key)
    setStatus(null)
    try {
      done(await work())
      return true
    } catch (error) {
      setStatus({ tone: 'error', message: messageOf(error, fallback) })
      return false
    } finally {
      setBusy(null)
    }
  }

  const showSaved = (action: string) => (saved: WebhookSaved) => {
    setView(saved.view)
    if (saved.token) setToken({ action, value: saved.token })
  }

  const server = serverEdits ?? (view ? serverDraftOf(view) : null)

  const saveServer = () => {
    if (!server) return
    void run('server', () => saveWebhookServer(libraryId, { enabled: server.enabled, port: Number(server.port), network: server.network }), (next) => {
      setView(next)
      setServerEdits(null)
    }, 'No se pudo guardar el webhook.')
  }

  const saveDraft = () => {
    if (!draft) return
    const name = draft.name
    void run('action', () => saveWebhookAction(libraryId, draft), showSaved(name), 'No se pudo guardar la acción.').then((saved) => {
      if (saved) setDraft(null)
    })
  }

  const toggleAction = (action: WebhookAction, enabled: boolean) => {
    void run(`toggle:${action.id}`, () => saveWebhookAction(libraryId, { actionId: action.id, name: action.name, instructions: action.instructions, enabled }), (saved) => setView(saved.view), 'No se pudo guardar la acción.')
  }

  const copyToken = async () => {
    if (!token) return
    try {
      await navigator.clipboard.writeText(token.value)
      setStatus({ tone: 'success', message: 'Token copiado.' })
    } catch {
      setStatus({ tone: 'error', message: 'No se pudo copiar: seleccioná el token y copialo a mano.' })
    }
  }

  const state = view?.state ?? 'off'
  const dirty = Boolean(view && server && !sameServer(server, view))
  const actions = view?.actions ?? []
  const calls = view?.calls ?? []
  const footerTone: SettingsTone = dirty ? 'idle' : state === 'listening' ? 'success' : state === 'error' ? 'error' : state === 'starting' ? 'loading' : 'idle'
  const footerMessage = dirty
    ? 'Hay cambios sin guardar.'
    : state === 'listening'
      ? `Escuchando en el puerto ${view?.port}.`
      : view?.message ?? 'El webhook está apagado: ningún programa puede llamar a Notia.'
  const exampleAction = actions[0]?.name ?? 'Alerta de seguridad'

  return (
    <>
      <SettingsCard>
        <div className="notia-settings-mail-gcp-head">
          <span className="notia-settings-mail-icon" data-provider="webhook" aria-hidden="true"><Webhook size={18} strokeWidth={1.8} /></span>
          <div className="notia-settings-mail-text">
            <div className="notia-settings-mail-name">
              <span className="notia-settings-block-title">Webhook</span>
              <SettingsBadge tone={state === 'listening' ? 'accent' : undefined} icon={<span className="notia-settings-mail-dot" aria-hidden="true" />}>
                {STATE_LABEL[state]}
              </SettingsBadge>
            </div>
            <div className="notia-settings-row-description">
              Otros programas le piden cosas a Notia con <code>POST /webhook</code>, nombrando una acción registrada. Notia lo hace como un pedido tuyo y te responde por Telegram.
            </div>
          </div>
        </div>
        {server ? (
          <>
            <SettingsRow label="Activo" description="Abre el puerto en esta PC mientras Notia está abierto." inline>
              <SettingsSwitch label="Webhook activo" checked={server.enabled} disabled={busy !== null} onChange={(enabled) => setServerEdits({ ...server, enabled })} />
            </SettingsRow>
            <SettingsRow label="Puerto" htmlFor="notia-webhook-port" inline>
              <input
                id="notia-webhook-port"
                className="notia-settings-field notia-settings-field--number"
                type="number"
                inputMode="numeric"
                min={1024}
                max={65535}
                value={server.port}
                disabled={busy !== null}
                onChange={(event) => setServerEdits({ ...server, port: event.target.value })}
              />
            </SettingsRow>
            <SettingsRow
              label="Llamadas desde la red"
              description="Apagado, solo pueden llamar los programas de esta PC. Prendido, también otros equipos de tu red: permití Notia en el Firewall de Windows. La conexión no va cifrada."
              inline
            >
              <SettingsSwitch label="Llamadas desde la red" checked={server.network} disabled={busy !== null} onChange={(network) => setServerEdits({ ...server, network })} />
            </SettingsRow>
            {view && view.urls.length > 0 ? (
              <SettingsRow label="Direcciones" description={view.urls.map((url) => <span key={url} className="notia-settings-path notia-webhook-url">{url}</span>)} />
            ) : null}
          </>
        ) : null}
        <SettingsFooter tone={footerTone} message={footerMessage}>
          <NotiaButton variant="primary" disabled={!dirty || busy !== null} onClick={saveServer}>
            {busy === 'server' ? 'Guardando…' : 'Guardar'}
          </NotiaButton>
        </SettingsFooter>
      </SettingsCard>

      {view && view.enabled && !view.telegram ? (
        <SettingsNotice tone="error">Telegram no está activo en esta PC: Notia rechaza las llamadas hasta que actives el bot en Configuración → Telegram, porque responde por ahí.</SettingsNotice>
      ) : null}
      {status ? <SettingsNotice tone={status.tone}>{status.message}</SettingsNotice> : null}

      {token ? (
        <SettingsCard>
          <div className="notia-settings-block notia-webhook-token">
            <div className="notia-settings-row-label"><span>Token de «{token.action}»</span></div>
            <div className="notia-settings-row-description">Guardalo ahora en el programa que va a llamar: Notia no lo vuelve a mostrar. Si lo perdés, creá uno nuevo.</div>
            <code className="notia-settings-path notia-webhook-token-value">{token.value}</code>
            <div className="notia-webhook-row-actions">
              <NotiaButton variant="ghost" onClick={() => setToken(null)}>Listo</NotiaButton>
              <NotiaButton variant="primary" onClick={() => { void copyToken() }}><Copy size={14} aria-hidden="true" /> Copiar token</NotiaButton>
            </div>
          </div>
        </SettingsCard>
      ) : null}

      <SettingsCard>
        <SettingsRow
          emphasis
          label="Acciones"
          description="Solo se puede llamar con una acción registrada acá y su token. Las instrucciones de la acción valen más que lo que pida el programa."
        >
          <NotiaButton variant="ghost" disabled={draft !== null} onClick={() => setDraft({ name: '', instructions: '', enabled: true })}>
            <Plus size={14} aria-hidden="true" /> Registrar acción
          </NotiaButton>
        </SettingsRow>
        {draft && !draft.actionId ? (
          <ActionForm draft={draft} saving={busy === 'action'} onChange={setDraft} onCancel={() => setDraft(null)} onSave={saveDraft} />
        ) : null}
        {actions.length === 0 && !draft ? <div className="notia-settings-empty">Todavía no hay acciones registradas.</div> : null}
        {actions.map((action) => (
          <div key={action.id} className="notia-webhook-action">
            <div className="notia-settings-list-row">
              <div className="notia-settings-mail-text">
                <div className="notia-settings-list-name">{action.name}</div>
                <div className="notia-settings-row-description">
                  {action.enabled ? 'Activa' : 'Pausada'}
                  {action.lastCallMs ? ` · última llamada ${timeFormat.format(new Date(action.lastCallMs))}` : ' · sin llamadas'}
                </div>
              </div>
              <SettingsSwitch label={`Acción activa: ${action.name}`} checked={action.enabled} disabled={busy !== null} onChange={(enabled) => toggleAction(action, enabled)} />
            </div>
            {action.instructions && draft?.actionId !== action.id ? <p className="notia-settings-note notia-webhook-instructions">{action.instructions}</p> : null}
            {draft?.actionId === action.id ? (
              <ActionForm draft={draft} saving={busy === 'action'} onChange={setDraft} onCancel={() => setDraft(null)} onSave={saveDraft} />
            ) : (
              <div className="notia-webhook-row-actions" role="group" aria-label={`Opciones de ${action.name}`}>
                <NotiaButton variant="ghost" disabled={busy !== null || draft !== null} onClick={() => setDraft({ actionId: action.id, name: action.name, instructions: action.instructions, enabled: action.enabled })}>
                  Editar
                </NotiaButton>
                <NotiaButton
                  variant="ghost"
                  disabled={busy !== null}
                  onClick={() => { void run(`token:${action.id}`, () => renewWebhookToken(libraryId, action.id), showSaved(action.name), 'No se pudo crear el token.') }}
                >
                  {busy === `token:${action.id}` ? 'Creando…' : 'Nuevo token'}
                </NotiaButton>
                {confirmDelete === action.id ? (
                  <>
                    <NotiaButton variant="ghost" onClick={() => setConfirmDelete(null)}>Cancelar</NotiaButton>
                    <NotiaButton
                      variant="danger"
                      disabled={busy !== null}
                      onClick={() => { void run(`delete:${action.id}`, () => deleteWebhookAction(libraryId, action.id), setView, 'No se pudo eliminar la acción.').then(() => setConfirmDelete(null)) }}
                    >
                      Eliminar «{action.name}»
                    </NotiaButton>
                  </>
                ) : (
                  <NotiaButton variant="danger" disabled={busy !== null} onClick={() => setConfirmDelete(action.id)}>Eliminar</NotiaButton>
                )}
              </div>
            )}
          </div>
        ))}
      </SettingsCard>

      {calls.length > 0 ? (
        <SettingsCard>
          <SettingsRow emphasis label="Últimas llamadas" description="Las de esta PC desde que Notia se abrió." />
          {calls.map((call) => (
            <div key={call.id} className="notia-settings-list-row notia-webhook-call">
              <div className="notia-settings-mail-text">
                <div className="notia-settings-list-name">
                  {call.action || 'Sin acción'}
                  <SettingsBadge tone={call.status === 'sent' ? 'accent' : undefined}>{CALL_LABEL[call.status]}</SettingsBadge>
                </div>
                <div className="notia-settings-row-description">
                  {timeFormat.format(new Date(call.receivedAtMs))}
                  {call.prompt ? ` · ${call.prompt}` : ''}
                </div>
                {call.detail ? <div className="notia-settings-row-description" data-tone={call.status === 'failed' || call.status === 'rejected' ? 'error' : undefined}>{call.detail}</div> : null}
              </div>
            </div>
          ))}
        </SettingsCard>
      ) : null}

      <p className="notia-settings-note">
        Así llama un programa (con el token de la acción):
      </p>
      <pre className="notia-settings-path notia-webhook-example">{`curl -X POST ${view?.exampleUrl ?? ''} \\
  -H "Authorization: Bearer <token>" \\
  -H "Content-Type: application/json" \\
  -d '{"action": "${exampleAction}", "prompt": "Avisale al usuario por Telegram que..."}'`}</pre>
      <p className="notia-settings-note">
        Notia contesta enseguida con <code>202</code> y hace el pedido en tu chat de Telegram; las confirmaciones te llegan ahí. Cada acción acepta hasta {view?.callsPerMinute} llamadas por minuto. Las acciones se guardan en <code>.notia/notiaConfig.json</code>, cifrado con tu contraseña, y de cada token solo queda su huella.
      </p>
    </>
  )
}

interface ActionFormProps {
  draft: WebhookActionDraft
  saving: boolean
  onChange: (draft: WebhookActionDraft) => void
  onCancel: () => void
  onSave: () => void
}

function ActionForm({ draft, saving, onChange, onCancel, onSave }: ActionFormProps) {
  const id = draft.actionId ?? 'new'
  return (
    <div className="notia-webhook-form">
      <label className="notia-settings-row-meta" htmlFor={`notia-webhook-name-${id}`}>Nombre (lo que el programa manda en «action»)</label>
      <input
        id={`notia-webhook-name-${id}`}
        className="notia-settings-field"
        value={draft.name}
        maxLength={80}
        placeholder="Alerta de seguridad"
        onChange={(event) => onChange({ ...draft, name: event.target.value })}
      />
      <label className="notia-settings-row-meta" htmlFor={`notia-webhook-instructions-${id}`}>Instrucciones para Notia (opcional)</label>
      <textarea
        id={`notia-webhook-instructions-${id}`}
        className="notia-settings-field notia-webhook-instructions-field"
        value={draft.instructions}
        maxLength={4000}
        rows={4}
        placeholder="Por ejemplo: Avisame por Telegram con el detalle de la alerta. No cambies nada por tu cuenta."
        onChange={(event) => onChange({ ...draft, instructions: event.target.value })}
      />
      <label className="notia-settings-wa-switch">
        <SettingsSwitch label="Acción activa" checked={draft.enabled} disabled={saving} onChange={(enabled) => onChange({ ...draft, enabled })} />
        <span>Activa</span>
      </label>
      <div className="notia-webhook-row-actions">
        <NotiaButton variant="ghost" disabled={saving} onClick={onCancel}>Cancelar</NotiaButton>
        <NotiaButton variant="primary" disabled={saving || !draft.name.trim()} onClick={onSave}>
          {saving ? 'Guardando…' : draft.actionId ? 'Guardar' : 'Registrar'}
        </NotiaButton>
      </div>
    </div>
  )
}
