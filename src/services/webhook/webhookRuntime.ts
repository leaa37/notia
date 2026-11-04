import { callBackend } from '../transport'

/*
 * The webhook of the host PC: other programs call `POST /webhook` with
 * `{ action, prompt }` and the token of a registered action. Rust keeps the
 * endpoint, the actions and the recent calls and decides everything; the
 * interface shows them and sends what the person changes.
 */

export type WebhookState = 'off' | 'starting' | 'listening' | 'error'

export type WebhookCallStatus = 'queued' | 'running' | 'sent' | 'silent' | 'failed' | 'rejected'

export interface WebhookAction {
  id: string
  name: string
  /** What the Owner wants done on each call, besides the call's prompt. */
  instructions: string
  enabled: boolean
  createdAtMs: number
  lastCallMs?: number
}

export interface WebhookCall {
  id: string
  action: string
  receivedAtMs: number
  status: WebhookCallStatus
  /** The start of the prompt; empty for a refused call. */
  prompt: string
  detail: string
}

export interface WebhookView {
  /** This device listens (the desktop host). */
  available: boolean
  unavailableReason?: string
  enabled: boolean
  port: number
  network: boolean
  state: WebhookState
  message?: string
  /** Addresses a program calls while it listens. */
  urls: string[]
  /** Where the example call points: the first address, or this PC's. */
  exampleUrl: string
  /** Runs one action may start per minute. */
  callsPerMinute: number
  /** The library's Telegram bot runs here, so calls can run. */
  telegram: boolean
  actions: WebhookAction[]
  calls: WebhookCall[]
}

export interface WebhookSaved {
  view: WebhookView
  /** The action's new token, shown only this once. */
  token?: string
  actionId?: string
}

export interface WebhookServerSettings {
  enabled: boolean
  port: number
  network: boolean
}

export interface WebhookActionDraft {
  /** Missing for a new action. */
  actionId?: string
  name: string
  instructions: string
  enabled: boolean
}

async function call<T>(command: string, payload: Record<string, unknown>, fallback: string): Promise<T> {
  try {
    return await callBackend<T>(command, { payload })
  } catch (error) {
    const detail = error && typeof error === 'object' ? error as { message?: unknown } : {}
    const message = typeof detail.message === 'string' && detail.message.trim()
      ? detail.message
      : error instanceof Error && error.message.trim() ? error.message : fallback
    throw new Error(message)
  }
}

export function readWebhook(libraryId: string): Promise<WebhookView> {
  return call('webhook_view', { libraryId }, 'No se pudo leer el webhook.')
}

export function saveWebhookServer(libraryId: string, settings: WebhookServerSettings): Promise<WebhookView> {
  return call('webhook_save_server', { libraryId, ...settings }, 'No se pudo guardar el webhook.')
}

/** Registers an action (the answer brings its token) or changes one. */
export function saveWebhookAction(libraryId: string, draft: WebhookActionDraft): Promise<WebhookSaved> {
  return call('webhook_save_action', { libraryId, ...draft }, 'No se pudo guardar la acción.')
}

/** A new token for the action; the previous one stops working. */
export function renewWebhookToken(libraryId: string, actionId: string): Promise<WebhookSaved> {
  return call('webhook_new_token', { libraryId, actionId }, 'No se pudo crear el token.')
}

export function deleteWebhookAction(libraryId: string, actionId: string): Promise<WebhookView> {
  return call('webhook_delete_action', { libraryId, actionId }, 'No se pudo eliminar la acción.')
}
