// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { WebhookAction, WebhookView } from '../../../services/webhook/webhookRuntime'

const runtime = vi.hoisted(() => ({
  readWebhook: vi.fn(),
  saveWebhookServer: vi.fn(),
  saveWebhookAction: vi.fn(),
  renewWebhookToken: vi.fn(),
  deleteWebhookAction: vi.fn(),
}))

vi.mock('../../../services/webhook/webhookRuntime', () => runtime)

const { WebhookSection } = await import('./WebhookSection')

const alert = (change: Partial<WebhookAction> = {}): WebhookAction => ({
  id: 'a1',
  name: 'Alerta de seguridad',
  instructions: '',
  enabled: true,
  createdAtMs: 0,
  ...change,
})
const view = (change: Partial<WebhookView> = {}): WebhookView => ({
  available: true,
  enabled: true,
  port: 52485,
  network: false,
  state: 'listening',
  urls: ['http://127.0.0.1:52485/webhook'],
  exampleUrl: 'http://127.0.0.1:52485/webhook',
  callsPerMinute: 10,
  telegram: true,
  actions: [],
  calls: [],
  ...change,
})

describe('WebhookSection', () => {
  beforeEach(() => {
    Object.values(runtime).forEach((mock) => mock.mockReset())
  })
  afterEach(cleanup)

  it('saves the endpoint only after a change', async () => {
    runtime.readWebhook.mockResolvedValue(view({ enabled: false, state: 'off', urls: [] }))
    runtime.saveWebhookServer.mockResolvedValue(view())
    render(<WebhookSection libraryId="lib" />)

    expect(await screen.findByText('Apagado')).toBeTruthy()
    const save = screen.getByRole('button', { name: 'Guardar' }) as HTMLButtonElement
    expect(save.disabled).toBe(true)
    fireEvent.click(screen.getByRole('switch', { name: 'Webhook activo' }))
    fireEvent.change(screen.getByLabelText('Puerto'), { target: { value: '52600' } })
    fireEvent.click(save)
    expect(runtime.saveWebhookServer).toHaveBeenCalledWith('lib', { enabled: true, port: 52600, network: false })
    expect(await screen.findByText('Escuchando')).toBeTruthy()
    expect(screen.getByText('http://127.0.0.1:52485/webhook')).toBeTruthy()
  })

  it('registers an action and shows its token once', async () => {
    runtime.readWebhook.mockResolvedValue(view())
    runtime.saveWebhookAction.mockResolvedValue({ view: view({ actions: [alert()] }), token: 'f00d', actionId: 'a1' })
    render(<WebhookSection libraryId="lib" />)

    expect(await screen.findByText('Todavía no hay acciones registradas.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Registrar acción' }))
    fireEvent.change(screen.getByLabelText('Nombre (lo que el programa manda en «action»)'), { target: { value: 'Alerta de seguridad' } })
    fireEvent.click(screen.getByRole('button', { name: 'Registrar' }))

    expect(runtime.saveWebhookAction).toHaveBeenCalledWith('lib', { name: 'Alerta de seguridad', instructions: '', enabled: true })
    expect(await screen.findByText('f00d')).toBeTruthy()
    expect(screen.getByText('Token de «Alerta de seguridad»')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Listo' }))
    await waitFor(() => expect(screen.queryByText('f00d')).toBeNull())
  })

  it('asks before deleting an action and warns without Telegram', async () => {
    runtime.readWebhook.mockResolvedValue(view({ telegram: false, actions: [alert()] }))
    runtime.deleteWebhookAction.mockResolvedValue(view({ telegram: false }))
    render(<WebhookSection libraryId="lib" />)

    expect(await screen.findByText(/Telegram no está activo en esta PC/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Eliminar' }))
    expect(runtime.deleteWebhookAction).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Eliminar «Alerta de seguridad»' }))
    expect(runtime.deleteWebhookAction).toHaveBeenCalledWith('lib', 'a1')
    expect(await screen.findByText('Todavía no hay acciones registradas.')).toBeTruthy()
  })

  it('shows why this device does not listen', async () => {
    runtime.readWebhook.mockResolvedValue(view({ available: false, unavailableReason: 'Este dispositivo es un cliente.' }))
    render(<WebhookSection libraryId="lib" />)
    expect(await screen.findByText('Este dispositivo es un cliente.')).toBeTruthy()
  })
})
