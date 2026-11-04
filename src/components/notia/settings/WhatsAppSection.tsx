import { useEffect, useState } from 'react'
import { MessageCircle, Search } from 'lucide-react'
import { NotiaButton } from '../../common/NotiaButton'
import { SettingsAvatar, SettingsBadge, SettingsCard, SettingsFooter, SettingsNotice, SettingsRow, SettingsSwitch, type SettingsTone } from './SettingsControls'
import {
  connectWhatsApp,
  disconnectWhatsApp,
  readWhatsApp,
  saveWhatsAppChat,
  type WhatsAppChat,
  type WhatsAppChatSettings,
  type WhatsAppView,
} from '../../../services/whatsapp/whatsappRuntime'

const LINK_STEPS = [
  'Abrí WhatsApp en tu teléfono.',
  <>Entrá a <b>Dispositivos vinculados</b> y tocá <b>Vincular un dispositivo</b>.</>,
  'Escaneá este código con la cámara del teléfono.',
]

/** How often the view is read again: fast while the link changes. */
function refreshDelay(view: WhatsAppView | null): number | null {
  if (!view || !view.available) return null
  if (view.state === 'starting' || view.state === 'qr' || view.state === 'closed') return 2000
  return view.state === 'open' ? 10000 : null
}

const STATE_LABEL: Record<WhatsAppView['state'], string> = {
  off: 'Desconectado',
  starting: 'Conectando',
  qr: 'Esperando el código',
  open: 'Conectado',
  closed: 'Reconectando',
  error: 'Sin conexión',
}

const timeFormat = new Intl.DateTimeFormat('es-AR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' })

function messageOf(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback
}

function settingsOf(chat: WhatsAppChat): WhatsAppChatSettings {
  return { chatId: chat.id, read: chat.read, write: chat.write, autoReply: chat.autoReply, prompt: chat.prompt }
}

interface WhatsAppSectionProps {
  /** Active library; the link and the chats' settings belong to it. */
  libraryId: string | null
}

/**
 * «WhatsApp»: links the Owner's WhatsApp with a QR and lists its chats, each
 * with what the assistant may do there (read, write), its auto reply and the
 * prompt that reply follows. Rust keeps the link and decides everything.
 */
export function WhatsAppSection({ libraryId }: WhatsAppSectionProps) {
  const [view, setView] = useState<WhatsAppView | null>(null)
  const [query, setQuery] = useState('')
  /** Restarts the reading of the view after a change. */
  const [generation, setGeneration] = useState(0)
  const [busy, setBusy] = useState<'connect' | 'disconnect' | null>(null)
  const [savingChat, setSavingChat] = useState<string | null>(null)
  const [promptChat, setPromptChat] = useState<string | null>(null)
  const [promptDraft, setPromptDraft] = useState('')
  const [status, setStatus] = useState<{ tone: SettingsTone; message: string } | null>(null)

  useEffect(() => {
    if (!libraryId) return
    let active = true
    let timer: number | undefined
    const load = async () => {
      let next: WhatsAppView | null = null
      try {
        next = await readWhatsApp(libraryId, query)
        if (active) setView(next)
      } catch (error) {
        if (active) setStatus({ tone: 'error', message: messageOf(error, 'No se pudo leer el estado de WhatsApp.') })
      }
      const delay = refreshDelay(next)
      if (active && delay !== null) timer = window.setTimeout(() => { void load() }, delay)
    }
    // The search waits for the person to stop typing.
    const start = window.setTimeout(() => { void load() }, query ? 300 : 0)
    return () => {
      active = false
      window.clearTimeout(start)
      window.clearTimeout(timer)
    }
  }, [libraryId, query, generation])

  if (!libraryId) {
    return <SettingsNotice tone="idle">Abrí una biblioteca para conectar WhatsApp.</SettingsNotice>
  }

  const runLink = async (kind: 'connect' | 'disconnect') => {
    setBusy(kind)
    setStatus(null)
    try {
      setView(await (kind === 'connect' ? connectWhatsApp(libraryId, query) : disconnectWhatsApp(libraryId, query)))
      setGeneration((value) => value + 1)
    } catch (error) {
      setStatus({ tone: 'error', message: messageOf(error, 'No se pudo completar la operación.') })
    } finally {
      setBusy(null)
    }
  }

  const saveChat = async (settings: WhatsAppChatSettings) => {
    setSavingChat(settings.chatId)
    setStatus(null)
    try {
      setView(await saveWhatsAppChat(libraryId, settings, query))
      return true
    } catch (error) {
      setStatus({ tone: 'error', message: messageOf(error, 'No se pudo guardar el chat.') })
      return false
    } finally {
      setSavingChat(null)
    }
  }

  const openPrompt = (chat: WhatsAppChat) => {
    if (promptChat === chat.id) {
      setPromptChat(null)
      return
    }
    setPromptChat(chat.id)
    setPromptDraft(chat.prompt)
  }

  if (view && !view.available) {
    return <SettingsNotice tone="idle">{view.unavailableReason}</SettingsNotice>
  }

  const state = view?.state ?? 'off'
  const chats = view?.chats ?? []
  const linked = view?.enabled ?? false
  const footerTone: SettingsTone = state === 'open' ? 'success' : state === 'error' ? 'error' : state === 'off' ? 'idle' : 'loading'
  const footerMessage = state === 'open'
    ? 'El asistente lee y escribe solo en los chats que habilites abajo.'
    : state === 'qr'
      ? 'Esperando que escanees el código…'
      : view?.message ?? (state === 'off' ? 'WhatsApp no está vinculado a Notia.' : 'Conectando con WhatsApp…')

  return (
    <>
      <SettingsCard>
        <div className="notia-settings-mail-gcp-head">
          <span className="notia-settings-mail-icon" data-provider="whatsapp" aria-hidden="true"><MessageCircle size={18} strokeWidth={1.8} /></span>
          <div className="notia-settings-mail-text">
            <div className="notia-settings-mail-name">
              <span className="notia-settings-block-title">WhatsApp</span>
              <SettingsBadge tone={state === 'open' ? 'accent' : undefined} icon={<span className="notia-settings-mail-dot" aria-hidden="true" />}>
                {STATE_LABEL[state]}
              </SettingsBadge>
            </div>
            <div className="notia-settings-row-description">
              {state === 'open' && view?.account
                ? [view.account.name, view.account.number].filter(Boolean).join(' · ')
                : 'Notia se vincula como un dispositivo más de tu WhatsApp, en la PC host. La primera vez instala Baileys con npm (necesita Node.js 20).'}
            </div>
          </div>
        </div>
        {state === 'qr' && view?.qrSvg ? (
          <div className="notia-settings-wa-link">
            <img className="notia-settings-wa-qr" src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(view.qrSvg)}`} alt="Código QR para vincular WhatsApp" />
            <div className="notia-settings-mail-steps">
              <div className="notia-settings-mail-steps-title">Cómo vincularlo</div>
              <ol>
                {LINK_STEPS.map((step, index) => (
                  <li key={index}><span aria-hidden="true">{index + 1}</span><span>{step}</span></li>
                ))}
              </ol>
            </div>
          </div>
        ) : null}
        <SettingsFooter tone={footerTone} message={footerMessage}>
          {!linked || state === 'error' ? (
            <NotiaButton variant="primary" disabled={busy !== null} onClick={() => { void runLink('connect') }}>
              {busy === 'connect' ? 'Conectando…' : state === 'error' ? 'Reintentar' : 'Conectar'}
            </NotiaButton>
          ) : null}
          {linked ? (
            <NotiaButton variant="danger" disabled={busy !== null} onClick={() => { void runLink('disconnect') }}>
              {busy === 'disconnect' ? 'Desconectando…' : state === 'qr' || state === 'starting' ? 'Cancelar' : 'Desconectar'}
            </NotiaButton>
          ) : null}
        </SettingsFooter>
      </SettingsCard>

      {status ? <SettingsNotice tone={status.tone}>{status.message}</SettingsNotice> : null}

      {chats.length > 0 || query ? (
        <SettingsCard>
          <SettingsRow
            emphasis
            label="Chats"
            description="Lectura deja que el asistente lea el chat y sus imágenes; Escritura, que escriba y mande archivos (siempre te pide confirmación). Con Autorespuesta, Notia contesta sola con el prompt del chat, un minuto después del último mensaje, leyendo todo lo que llegó."
          />
          <div className="notia-settings-wa-search">
            <label className="notia-settings-search">
              <Search size={14} aria-hidden="true" />
              <input
                type="search"
                value={query}
                placeholder="Buscar por nombre o número"
                aria-label="Buscar chats de WhatsApp"
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            <span className="notia-settings-row-meta">
              {view && view.totalChats > chats.length ? `${chats.length} de ${view.totalChats}` : `${chats.length} ${chats.length === 1 ? 'chat' : 'chats'}`}
            </span>
          </div>
          {chats.length === 0 ? (
            <div className="notia-settings-mail-empty">
              <div className="notia-settings-list-name">Ningún chat coincide</div>
            </div>
          ) : chats.map((chat, index) => {
            const saving = savingChat === chat.id
            const toggle = (change: Partial<WhatsAppChatSettings>) => { void saveChat({ ...settingsOf(chat), ...change }) }
            return (
              <div key={chat.id} className="notia-settings-wa-chat">
                <div className="notia-settings-list-row notia-settings-wa-chat-row">
                  <SettingsAvatar name={chat.name} index={index} small />
                  <div className="notia-settings-mail-text">
                    <div className="notia-settings-list-name">{chat.name}</div>
                    <div className="notia-settings-row-description">
                      {chat.group ? 'Grupo' : 'Chat'}
                      {chat.lastMessageMs > 0 ? ` · ${timeFormat.format(new Date(chat.lastMessageMs))}` : ''}
                    </div>
                  </div>
                </div>
                <div className="notia-settings-wa-controls" role="group" aria-label={`Permisos de ${chat.name}`}>
                  <label className="notia-settings-wa-switch">
                    <SettingsSwitch label={`Lectura: ${chat.name}`} checked={chat.read} disabled={saving} onChange={(read) => toggle({ read })} />
                    <span>Lectura</span>
                  </label>
                  <label className="notia-settings-wa-switch">
                    <SettingsSwitch label={`Escritura: ${chat.name}`} checked={chat.write} disabled={saving} onChange={(write) => toggle({ write })} />
                    <span>Escritura</span>
                  </label>
                  <label className="notia-settings-wa-switch">
                    <SettingsSwitch label={`Autorespuesta: ${chat.name}`} checked={chat.autoReply} disabled={saving} onChange={(autoReply) => toggle({ autoReply })} />
                    <span>Autorespuesta</span>
                  </label>
                  <NotiaButton
                    variant="ghost"
                    className="notia-settings-wa-prompt-toggle"
                    aria-expanded={promptChat === chat.id}
                    aria-label={`Prompt de ${chat.name}`}
                    onClick={() => openPrompt(chat)}
                  >
                    {chat.prompt ? 'Editar prompt' : 'Prompt'}
                  </NotiaButton>
                </div>
                {promptChat === chat.id ? (
                  <div className="notia-settings-wa-prompt">
                    <label className="notia-settings-row-meta" htmlFor={`notia-settings-wa-prompt-${index}`}>
                      Instrucciones de la autorespuesta en este chat
                    </label>
                    <textarea
                      id={`notia-settings-wa-prompt-${index}`}
                      className="notia-settings-field notia-settings-wa-prompt-field"
                      value={promptDraft}
                      maxLength={4000}
                      rows={5}
                      placeholder="Por ejemplo: Sos el asistente de mi local. Abrimos de lunes a viernes de 9 a 18. Si preguntan precios, deciles que les paso la lista a la tarde."
                      onChange={(event) => setPromptDraft(event.target.value)}
                    />
                    <div className="notia-settings-wa-prompt-actions">
                      <NotiaButton variant="ghost" disabled={saving} onClick={() => setPromptChat(null)}>Cancelar</NotiaButton>
                      <NotiaButton
                        variant="primary"
                        disabled={saving || promptDraft === chat.prompt}
                        onClick={() => {
                          void saveChat({ ...settingsOf(chat), prompt: promptDraft }).then((saved) => { if (saved) setPromptChat(null) })
                        }}
                      >
                        {saving ? 'Guardando…' : 'Guardar'}
                      </NotiaButton>
                    </div>
                  </div>
                ) : null}
              </div>
            )
          })}
        </SettingsCard>
      ) : null}

      <p className="notia-settings-note">
        La sesión de WhatsApp, los chats y sus últimos mensajes se guardan solo en la PC host. Los permisos y los prompts de cada chat se guardan en <code>.notia/notiaConfig.json</code>, cifrado con tu contraseña.
      </p>
    </>
  )
}
