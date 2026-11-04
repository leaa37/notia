// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { ConnectionView } from '../../../services/connection/connectionRuntime'

const runtime = vi.hoisted(() => ({
  fetchConnection: vi.fn(),
  saveConnection: vi.fn(),
  testHostConnection: vi.fn(),
  pickCopyFolder: vi.fn(),
  syncCopyNow: vi.fn(),
}))

vi.mock('../../../services/connection/connectionRuntime', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../services/connection/connectionRuntime')>()),
  ...runtime,
}))

const { RunModeSection } = await import('./RunModeSection')

const view = (overrides: Partial<ConnectionView> = {}): ConnectionView => ({
  mode: 'host',
  hostAddress: '',
  canHost: true,
  server: { listening: true, port: 52480, error: null },
  link: { state: 'unknown', library: null, platform: null, signedIn: false, message: null },
  copyNeedsFolder: false,
  copyFolder: null,
  copyReady: false,
  copy: null,
  progress: null,
  runsServices: true,
  statusInNotification: false,
  ...overrides,
})

describe('RunModeSection', () => {
  beforeEach(() => {
    Object.values(runtime).forEach((mock) => mock.mockReset())
  })
  afterEach(cleanup)

  it('shows the port a host listens on and that it runs the services', async () => {
    runtime.fetchConnection.mockResolvedValue(view())
    render(<RunModeSection />)
    expect(await screen.findByText('Escuchando')).toBeTruthy()
    expect(screen.getByText('52480')).toBeTruthy()
    expect(screen.getByRole('radio', { name: /Host/ }).getAttribute('aria-checked')).toBe('true')
    expect(screen.getByText('En esta computadora')).toBeTruthy()
  })

  it('becomes a client once the address of the host is saved', async () => {
    runtime.fetchConnection.mockResolvedValue(view())
    runtime.saveConnection.mockResolvedValue({ connection: view({ mode: 'client', hostAddress: '192.168.0.10:52480' }), reload: false })
    runtime.testHostConnection.mockResolvedValue({ ok: true, library: 'gaia', latencyMs: 4, message: null })
    render(<RunModeSection />)
    fireEvent.click(await screen.findByRole('radio', { name: /Cliente/ }))
    expect(runtime.saveConnection).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Dirección del host'), { target: { value: '192.168.0.10' } })
    fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))
    await waitFor(() => expect(runtime.saveConnection).toHaveBeenCalledWith({
      mode: 'client',
      hostAddress: '192.168.0.10',
      trustNewCertificate: true,
    }))
    expect(await screen.findByText('Conectado')).toBeTruthy()
  })

  it('reports a host that does not answer and that the copy keeps working', async () => {
    const offline = view({
      mode: 'client',
      hostAddress: '192.168.0.10:52480',
      runsServices: false,
      link: { state: 'offline', library: 'gaia', platform: null, signedIn: false, message: null },
    })
    runtime.fetchConnection.mockResolvedValue(offline)
    runtime.testHostConnection.mockResolvedValue({ ok: false, library: null, latencyMs: 0, message: 'No se pudo contactar al host de Notia.' })
    render(<RunModeSection />)
    expect(await screen.findByText('No responde')).toBeTruthy()
    expect(screen.getByRole('alert').textContent).toContain('No se pudo contactar')
    expect(screen.getByText(/trabajás con la copia local/)).toBeTruthy()
    expect(screen.getByText('En otra computadora')).toBeTruthy()
  })

  it('syncs the copy now and shows the last sync', async () => {
    const client = view({
      mode: 'client',
      hostAddress: 'casa:52480',
      link: { state: 'online', library: 'gaia', platform: 'windows', signedIn: true, message: null },
    })
    const synced = { ...client, copy: { downloaded: 1, uploaded: 0, deleted: 0, rowsUploaded: 0, rowsDownloaded: 2, skipped: [], atMs: Date.UTC(2026, 10, 3, 15, 30), error: null } }
    runtime.fetchConnection.mockResolvedValueOnce(client).mockResolvedValue(synced)
    runtime.testHostConnection.mockResolvedValue({ ok: true, library: 'gaia', latencyMs: 2, message: null })
    runtime.syncCopyNow.mockResolvedValue(synced.copy)
    render(<RunModeSection />)
    expect(await screen.findByText('Todavía no se sincronizó.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Sincronizar ahora' }))
    expect(runtime.syncCopyNow).toHaveBeenCalledTimes(1)
    expect(await screen.findByText(/^Sincronizado a las/)).toBeTruthy()
  })

  it('asks Android for the folder of the copy before becoming a client', async () => {
    const android = view({ canHost: false, copyNeedsFolder: true })
    runtime.fetchConnection.mockResolvedValue(android)
    runtime.pickCopyFolder.mockResolvedValue({ ...android, copyFolder: 'NotiaCopia' })
    runtime.saveConnection.mockResolvedValue({ connection: { ...android, mode: 'client', copyFolder: 'NotiaCopia' }, reload: true })
    render(<RunModeSection />)
    fireEvent.click(await screen.findByRole('radio', { name: /Cliente/ }))
    expect(screen.getByText('Sin elegir')).toBeTruthy()
    // Android never runs the services: there is no row for them.
    expect(screen.queryByText('Servicios de la biblioteca')).toBeNull()
    fireEvent.change(screen.getByLabelText('Dirección del host'), { target: { value: 'casa' } })
    expect((screen.getByRole('button', { name: 'Guardar' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Elegir carpeta' }))
    expect(await screen.findByText('NotiaCopia')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))
    await waitFor(() => expect(runtime.saveConnection).toHaveBeenCalledWith({ mode: 'client', hostAddress: 'casa', trustNewCertificate: true }))
    expect(runtime.pickCopyFolder).toHaveBeenCalledTimes(1)
  })
})
