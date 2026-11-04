import { callBackend } from '../transport'

/**
 * «Modo de ejecución» of this device (`connection.rs`): Host serves its
 * library to the devices that sync with it; Client works on its copy of a
 * host's library and syncs it whenever the host answers. The backend
 * stores and applies the mode, decides where the window opens and which
 * computer runs the library's services, and reports how the sync is. The
 * interface only shows it.
 */
export type RunMode = 'host' | 'client'
export type HostLinkState = 'unknown' | 'online' | 'offline'

export interface HostServerStatus {
  listening: boolean
  port: number
  error: string | null
}

export interface HostLink {
  state: HostLinkState
  /** Library the host serves. */
  library: string | null
  /** Operating system of the host. */
  platform: string | null
  /** The host accepts the Owner's session: the copy syncs. */
  signedIn: boolean
  message: string | null
}

/** Last sync of the copy. */
export interface CopyStatus {
  downloaded: number
  uploaded: number
  deleted: number
  /** Rows of the database this device sent to the host, and taken from it. */
  rowsUploaded: number
  rowsDownloaded: number
  /** Files that did not travel (too large, or the host did not give them). */
  skipped: string[]
  atMs: number
  error: string | null
}

/** How far the running sync went. */
export interface CopyProgress {
  done: number
  total: number
}

export interface ConnectionView {
  mode: RunMode
  hostAddress: string
  /** Android has no server: it can only use its own library or sync with a host. */
  canHost: boolean
  server: HostServerStatus
  link: HostLink
  /** Android: the copy lives in a folder the person chooses. */
  copyNeedsFolder: boolean
  /** Name of the chosen folder of the copy (Android). */
  copyFolder: string | null
  /** The copy is ready: the window opens on it. */
  copyReady: boolean
  copy: CopyStatus | null
  progress: CopyProgress | null
  /** This device runs Telegram, WhatsApp, the webhook and the other services of the library. */
  runsServices: boolean
  /** The state of the sync goes in a system notification (Android): the window shows no notice. */
  statusInNotification: boolean
}

export interface SavedConnection {
  connection: ConnectionView
  /** The interface reloads: it opens another library. */
  reload: boolean
}

/** Where the window of a client opens (`client_open`). */
export type ClientOpening = 'copy' | 'first-sync'

export interface OpenedClient {
  opening: ClientOpening
  /** Why the copy did not open, when it is ready but failed. */
  message: string | null
  connection: ConnectionView
}

export interface HostProbe {
  ok: boolean
  library: string | null
  latencyMs: number
  message: string | null
}

/** Event of the backend after each sync of the copy. */
export const COPY_SYNC_EVENT = 'notia:copy-sync'

export function fetchConnection(): Promise<ConnectionView> {
  return callBackend<ConnectionView>('connection_settings')
}

export function saveConnection(input: {
  mode: RunMode
  hostAddress: string
  trustNewCertificate?: boolean
}): Promise<SavedConnection> {
  return callBackend<SavedConnection>('save_connection_settings', { payload: input })
}

/** «Probar conexión»: the saved host, or `hostAddress` before saving it. */
export function testHostConnection(hostAddress?: string): Promise<HostProbe> {
  return callBackend<HostProbe>('test_host_connection', { payload: hostAddress ? { hostAddress } : {} })
}

/**
 * Opens the window of a client: on its copy, at once, with or without the
 * host. Without a copy yet, the window signs in with the host first.
 */
export function openClient(): Promise<OpenedClient> {
  return callBackend<OpenedClient>('client_open')
}

/** Android: picks the folder of the copy (empty, or an earlier copy). */
export function pickCopyFolder(): Promise<ConnectionView> {
  return callBackend<ConnectionView>('pick_copy_folder')
}

/** Syncs the copy now (the last change of each file and each row wins). */
export function syncCopyNow(): Promise<CopyStatus> {
  return callBackend<CopyStatus>('sync_copy_now')
}
