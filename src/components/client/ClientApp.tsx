import { lazy, Suspense, useCallback, useEffect, useState, type FormEvent, type ReactNode } from 'react'
// The screens before the app (sign-in, first copy) use the app's styles and
// theme tokens, which otherwise load only with the app.
import '../../styles/notia.css'
import { subscribeBackend } from '../../services/transport'
import {
  COPY_SYNC_EVENT,
  fetchConnection,
  openClient,
  saveConnection,
  type ConnectionView,
} from '../../services/connection/connectionRuntime'
import { authErrorMessage, fetchAppAuthStatus, type AppAuthStatus } from '../../services/auth/appAuthRuntime'
import { LoginScreen } from '../notia/auth/LoginScreen'

const App = lazy(() => import('../../App'))

type Stage =
  | { kind: 'checking' }
  /** The app works on the copy, with or without the host. */
  | { kind: 'copy' }
  /** The first copy: the Owner signs in with the host. */
  | { kind: 'sign-in'; status: AppAuthStatus & { libraryId: string } }
  /** The first copy is being made. */
  | { kind: 'copying' }
  /** The first copy cannot start: the host does not answer. */
  | { kind: 'unreachable'; message: string }

/** How often the first copy screen reads how far the sync went. */
const PROGRESS_INTERVAL_MS = 2000

function prefersLightTheme(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: light)').matches
}

function ClientScreen({ library, children }: { library?: string | null; children: ReactNode }) {
  return (
    <div className={`notia-app-shell ${prefersLightTheme() ? 'notia-theme-light' : 'notia-theme-dark'}`}>
      <div className="notia-login-screen">
        <main className="notia-login-card">
          <div className="notia-login-brand">
            <svg width="28" height="28" viewBox="0 0 28 28" aria-hidden="true">
              <path d="M8 4h9l5 5v13a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" />
              <path d="M17 4v5h5" />
              <path d="M10 15h8M10 19h5" />
            </svg>
            <span className="notia-login-brand-name">Notia</span>
            {library ? <span className="notia-login-library">{library}</span> : null}
          </div>
          {children}
        </main>
      </div>
    </div>
  )
}

/** The first copy cannot start: retry, point to another host or use this device. */
function UnreachableScreen({ connection, message, onRetry }: { connection: ConnectionView; message: string; onRetry: () => void }) {
  const [address, setAddress] = useState(connection.hostAddress)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const apply = async (input: Parameters<typeof saveConnection>[0]) => {
    setBusy(true)
    setError(null)
    try {
      await saveConnection(input)
      window.location.reload()
    } catch (reason) {
      setError(authErrorMessage(reason, 'No se pudo guardar el modo de ejecución.'))
      setBusy(false)
    }
  }

  const submitAddress = (event: FormEvent) => {
    event.preventDefault()
    if (!address.trim() || busy) return
    void apply({ mode: 'client', hostAddress: address, trustNewCertificate: true })
  }

  return (
    <ClientScreen>
      <form className="notia-login-body" onSubmit={submitAddress} noValidate aria-busy={busy}>
        <div className="notia-login-heading">
          <h1>Sin conexión con el host</h1>
          <p>{message}</p>
          <p>La primera vez, este dispositivo copia la biblioteca del host. Después funciona sin conexión y se sincroniza cuando el host responde.</p>
        </div>
        <div className="notia-login-fields">
          <div className="notia-login-field">
            <label htmlFor="client-host-address">Host</label>
            <input
              id="client-host-address"
              className="notia-login-input notia-client-address"
              type="text"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              placeholder="[host]:[puerto]"
              value={address}
              onChange={(event) => setAddress(event.target.value)}
            />
          </div>
        </div>
        <div className="notia-login-actions">
          {error ? <p className="notia-login-error" role="alert">{error}</p> : null}
          <button type="button" className="notia-login-primary" disabled={busy} onClick={onRetry}>Reintentar</button>
          {address.trim() && address.trim() !== connection.hostAddress ? (
            <button type="submit" className="notia-login-link" disabled={busy}>Guardar esta dirección</button>
          ) : null}
          <button
            type="button"
            className="notia-login-link"
            disabled={busy}
            onClick={() => { void apply({ mode: 'host', hostAddress: connection.hostAddress }) }}
          >
            {connection.canHost ? 'Usar este equipo como host' : 'Usar la biblioteca de este dispositivo'}
          </button>
        </div>
      </form>
    </ClientScreen>
  )
}

function progressText(connection: ConnectionView): string {
  const progress = connection.progress
  if (progress && progress.total > 0) {
    return `${progress.done} de ${progress.total === 1 ? '1 archivo' : `${progress.total} archivos`}.`
  }
  return 'Preparando la copia…'
}

/** The first copy: how far it went, until the copy opens. */
function CopyingScreen({ connection, onReady }: { connection: ConnectionView; onReady: () => void }) {
  const [view, setView] = useState(connection)

  useEffect(() => {
    let active = true
    const read = () => {
      void fetchConnection()
        .then((next) => { if (active) setView(next) })
        .catch(() => {})
    }
    const timer = window.setInterval(read, PROGRESS_INTERVAL_MS)
    let unsubscribe: (() => void) | null = null
    void subscribeBackend(COPY_SYNC_EVENT, read).then((stop) => {
      if (active) unsubscribe = stop
      else stop()
    })
    return () => {
      active = false
      window.clearInterval(timer)
      unsubscribe?.()
    }
  }, [])

  const ready = view.copyReady
  useEffect(() => {
    if (ready) onReady()
  }, [ready, onReady])

  const error = view.copy?.error ?? null
  return (
    <ClientScreen library={view.link.library}>
      <div className="notia-login-body">
        <div className="notia-login-heading">
          <h1>Copiando la biblioteca</h1>
          <p>Este dispositivo guarda una copia de la biblioteca de {view.hostAddress} para usarla también sin conexión. La primera vez puede tardar unos minutos.</p>
        </div>
        <p className="notia-login-text" role="status">{progressText(view)}</p>
        {error ? <p className="notia-login-error" role="alert">{error} Se vuelve a intentar sola.</p> : null}
      </div>
    </ClientScreen>
  )
}

/**
 * Window of a client (Settings → General → «Modo de ejecución»): it always
 * works on the copy of the host's library, and the backend syncs the copy
 * whenever the host answers. Only the first time, before there is a copy,
 * the Owner signs in with the host and the window waits for the first copy.
 */
export function ClientApp({ initial }: { initial: ConnectionView }) {
  const [connection, setConnection] = useState(initial)
  const [stage, setStage] = useState<Stage>({ kind: 'checking' })

  const open = useCallback(async () => {
    setStage({ kind: 'checking' })
    const opened = await openClient()
    setConnection(opened.connection)
    if (opened.opening === 'copy') {
      setStage({ kind: 'copy' })
      return
    }
    // The first copy: the backend asks the host for its sign-in.
    try {
      const status = await fetchAppAuthStatus(null)
      if (status.state === 'unlocked') {
        setStage({ kind: 'copying' })
      } else if (status.libraryId) {
        setStage({ kind: 'sign-in', status: { ...status, libraryId: status.libraryId } })
      } else {
        setStage({ kind: 'unreachable', message: opened.message ?? 'El host no tiene una biblioteca abierta.' })
      }
    } catch (reason) {
      setStage({ kind: 'unreachable', message: opened.message ?? authErrorMessage(reason, 'No se pudo conectar con el host.') })
    }
  }, [])

  const retry = useCallback(() => {
    void open().catch((reason: unknown) => {
      setStage({ kind: 'unreachable', message: authErrorMessage(reason, 'No se pudo conectar con el host.') })
    })
  }, [open])

  useEffect(() => {
    retry()
  }, [retry])

  if (stage.kind === 'copy') {
    return (
      <Suspense fallback={<ClientScreen><p className="notia-login-text" role="status">Cargando Notia…</p></ClientScreen>}>
        <App />
      </Suspense>
    )
  }
  if (stage.kind === 'sign-in') {
    return (
      <div className={`notia-app-shell ${prefersLightTheme() ? 'notia-theme-light' : 'notia-theme-dark'}`}>
        <LoginScreen status={stage.status} canRemember onUnlocked={() => setStage({ kind: 'copying' })} />
      </div>
    )
  }
  if (stage.kind === 'copying') {
    return <CopyingScreen connection={connection} onReady={retry} />
  }
  if (stage.kind === 'unreachable') {
    return <UnreachableScreen connection={connection} message={stage.message} onRetry={retry} />
  }
  return null
}
