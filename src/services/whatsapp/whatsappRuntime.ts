import { callBackend } from '../transport'

/*
 * The Owner's WhatsApp, linked to Notia on the host PC (Baileys). Rust keeps
 * the link, the chats and what each chat allows; the interface shows the
 * state, the QR to scan and the chats, and sends what the person changes.
 */

export type WhatsAppState = 'off' | 'starting' | 'qr' | 'open' | 'closed' | 'error'

export interface WhatsAppChat {
  id: string
  name: string
  group: boolean
  lastMessageMs: number
  read: boolean
  write: boolean
  autoReply: boolean
  /** Instructions the auto reply follows in this chat. */
  prompt: string
}

export interface WhatsAppView {
  /** This device links WhatsApp (the desktop host). */
  available: boolean
  unavailableReason?: string
  enabled: boolean
  state: WhatsAppState
  /** SVG of the QR to scan while `state` is `qr`. */
  qrSvg?: string
  account?: { name: string | null; number: string | null }
  message?: string
  chats: WhatsAppChat[]
  /** Chats that match the search, also beyond the listed ones. */
  totalChats: number
}

export interface WhatsAppChatSettings {
  chatId: string
  read: boolean
  write: boolean
  autoReply: boolean
  prompt: string
}

async function call(command: string, payload: Record<string, unknown>, fallback: string): Promise<WhatsAppView> {
  try {
    return await callBackend<WhatsAppView>(command, { payload })
  } catch (error) {
    const detail = error && typeof error === 'object' ? error as { message?: unknown } : {}
    const message = typeof detail.message === 'string' && detail.message.trim()
      ? detail.message
      : error instanceof Error && error.message.trim() ? error.message : fallback
    throw new Error(message)
  }
}

export function readWhatsApp(libraryId: string, query: string): Promise<WhatsAppView> {
  return call('whatsapp_view', { libraryId, query }, 'No se pudo leer el estado de WhatsApp.')
}

/** Turns the link on; the view then shows the QR to scan. */
export function connectWhatsApp(libraryId: string, query: string): Promise<WhatsAppView> {
  return call('whatsapp_connect', { libraryId, query }, 'No se pudo conectar WhatsApp.')
}

/** Unlinks Notia and deletes the session and the stored chats of the host. */
export function disconnectWhatsApp(libraryId: string, query: string): Promise<WhatsAppView> {
  return call('whatsapp_disconnect', { libraryId, query }, 'No se pudo desconectar WhatsApp.')
}

export function saveWhatsAppChat(libraryId: string, settings: WhatsAppChatSettings, query: string): Promise<WhatsAppView> {
  return call('whatsapp_set_chat', { libraryId, query, ...settings }, 'No se pudo guardar el chat.')
}
