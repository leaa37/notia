import { lazy, Suspense, useCallback, useEffect, useState, type FormEvent } from 'react'
// The screens before the app (sign-in, connection) use the app's styles and
// theme tokens, which otherwise load only with the app.
import '../../styles/notia.css'
import { installBackendTransport, subscribeBackend } from '../../services/transport'
import { createRemoteTransport, EVENTS_LOST_EVENT } from '../../services/transport/remoteTransport'
import {
  fetchRemoteCapabilities,
  fetchRemoteSession,
  isRemoteServer,
  loginRemote,
} from '../../services/transport/remoteSession'
import { NotiaButton } from '../common/NotiaButton'

// Loaded after the remote transport is installed: modules of the interface
// read which backend they talk to when they load.
const App = lazy(() => import('../../App'))

type Stage =
  | { kind: 'checking' }
  | { kind: 'unavailable' }
  | { kind: 'login' }
  | { kind: 'ready' }
  | { kind: 'failed'; message: string }

function messageOf(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback
}

function prefersLightTheme(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: light)').matches
}

function RemoteScreen({ children }: { children: React.ReactNode }) {
  return (
    <div className={`notia-app-shell ${prefersLightTheme() ? 'notia-theme-light' : 'notia-theme-dark'} notia-remote-screen`}>
      <main className="notia-remote-card">
        <h1 className="notia-remote-title">Notia</h1>
        {children}
      </main>
    </div>
  )
}

/**
 * Interface served by a headless Notia server to a browser: checks the
 * server, asks for the owner password and then loads the application with
 * the remote transport and the commands the server offers.
 */
export function RemoteApp() {
  const [stage, setStage] = useState<Stage>({ kind: 'checking' })
  const [password, setPassword] = useState('')
  const [loginError, setLoginError] = useState<string | null>(null)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [eventsLost, setEventsLost] = useState(false)

  const enter = useCallback(async () => {
    const capabilities = await fetchRemoteCapabilities()
    installBackendTransport(createRemoteTransport({
      capabilities,
      // The interface keeps state of the old session; start clean.
      onSessionExpired: () => window.location.reload(),
    }))
    // Missed events that the server no longer keeps: offer to reload
    // instead of reloading under the person's feet.
    void subscribeBackend(EVENTS_LOST_EVENT, () => setEventsLost(true))
    setStage({ kind: 'ready' })
  }, [])

  useEffect(() => {
    let current = true
    void (async () => {
      if (!await isRemoteServer()) {
        if (current) setStage({ kind: 'unavailable' })
        return
      }
      if (!await fetchRemoteSession()) {
        if (current) setStage({ kind: 'login' })
        return
      }
      if (current) await enter()
    })().catch((error: unknown) => {
      if (current) setStage({ kind: 'failed', message: messageOf(error, 'No se pudo conectar con el servidor de Notia.') })
    })
    return () => { current = false }
  }, [enter])

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!password || isSubmitting) return
    setIsSubmitting(true)
    setLoginError(null)
    try {
      await loginRemote(password)
      setPassword('')
      await enter()
    } catch (error) {
      setLoginError(messageOf(error, 'No se pudo iniciar sesión.'))
    } finally {
      setIsSubmitting(false)
    }
  }

  if (stage.kind === 'ready') {
    return (
      <>
        {eventsLost ? (
          <div className="notia-remote-banner" role="alert">
            <span>Se perdió la conexión con el servidor y puede haber cambios sin mostrar.</span>
            <NotiaButton variant="primary" size="sm" onClick={() => window.location.reload()}>Recargar</NotiaButton>
          </div>
        ) : null}
        <Suspense fallback={<RemoteScreen><p className="notia-remote-text" role="status">Cargando Notia…</p></RemoteScreen>}>
          <App />
        </Suspense>
      </>
    )
  }

  if (stage.kind === 'checking') {
    return <RemoteScreen><p className="notia-remote-text" role="status">Conectando con el servidor…</p></RemoteScreen>
  }

  if (stage.kind === 'unavailable' || stage.kind === 'failed') {
    return (
      <RemoteScreen>
        <p className="notia-remote-text" role="alert">
          {stage.kind === 'failed'
            ? stage.message
            : 'Esta dirección no es un servidor de Notia. Abrí la dirección que muestra `notia --headless` al iniciar.'}
        </p>
        <NotiaButton variant="secondary" onClick={() => window.location.reload()}>Reintentar</NotiaButton>
      </RemoteScreen>
    )
  }

  return (
    <RemoteScreen>
      <form className="notia-remote-form" onSubmit={(event) => { void submit(event) }}>
        <label className="notia-remote-label" htmlFor="notia-remote-password">Contraseña del servidor</label>
        <input
          id="notia-remote-password"
          className="notia-remote-input"
          type="password"
          autoComplete="current-password"
          autoFocus
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          disabled={isSubmitting}
        />
        {loginError ? <p className="notia-remote-error" role="alert">{loginError}</p> : null}
        <NotiaButton type="submit" variant="primary" disabled={!password || isSubmitting}>
          {isSubmitting ? 'Iniciando…' : 'Iniciar'}
        </NotiaButton>
      </form>
    </RemoteScreen>
  )
}
