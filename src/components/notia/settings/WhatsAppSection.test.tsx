// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { WhatsAppChat, WhatsAppView } from '../../../services/whatsapp/whatsappRuntime'

const runtime = vi.hoisted(() => ({
  readWhatsApp: vi.fn(),
  connectWhatsApp: vi.fn(),
  disconnectWhatsApp: vi.fn(),
  saveWhatsAppChat: vi.fn(),
}))

vi.mock('../../../services/whatsapp/whatsappRuntime', () => runtime)

const { WhatsAppSection } = await import('./WhatsAppSection')

const JUAN = '5491111111111@s.whatsapp.net'
const chat = (change: Partial<WhatsAppChat> = {}): WhatsAppChat => ({
  id: JUAN,
  name: 'Juan Pérez',
  group: false,
  lastMessageMs: 0,
  read: false,
  write: false,
  autoReply: false,
  prompt: '',
  ...change,
})
const view = (change: Partial<WhatsAppView> = {}): WhatsAppView => ({
  available: true,
  enabled: true,
  state: 'open',
  account: { name: 'Gabi', number: '+5491100000000' },
  chats: [chat()],
  totalChats: 1,
  ...change,
})

describe('WhatsAppSection', () => {
  beforeEach(() => {
    Object.values(runtime).forEach((mock) => mock.mockReset())
  })
  afterEach(cleanup)

  it('connects and shows the QR to scan', async () => {
    runtime.readWhatsApp.mockResolvedValue(view({ enabled: false, state: 'off', account: undefined, chats: [], totalChats: 0 }))
    runtime.connectWhatsApp.mockResolvedValue(view({ state: 'qr', qrSvg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>', account: undefined, chats: [], totalChats: 0 }))
    render(<WhatsAppSection libraryId="lib" />)

    expect(await screen.findByText('Desconectado')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))
    expect(runtime.connectWhatsApp).toHaveBeenCalledWith('lib', '')
    expect(await screen.findByAltText('Código QR para vincular WhatsApp')).toBeTruthy()
    expect(screen.getByText('Esperando que escanees el código…')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Cancelar' })).toBeTruthy()
  })

  it('saves what each chat allows and its prompt', async () => {
    runtime.readWhatsApp.mockResolvedValue(view())
    runtime.saveWhatsAppChat.mockImplementation(async (_library: string, settings: { read: boolean; prompt: string }) =>
      view({ chats: [chat({ read: settings.read, prompt: settings.prompt })] }))
    render(<WhatsAppSection libraryId="lib" />)

    expect(await screen.findByText('Gabi · +5491100000000')).toBeTruthy()
    fireEvent.click(screen.getByRole('switch', { name: 'Lectura: Juan Pérez' }))
    expect(runtime.saveWhatsAppChat).toHaveBeenCalledWith('lib', { chatId: JUAN, read: true, write: false, autoReply: false, prompt: '' }, '')
    await waitFor(() => expect((screen.getByRole('switch', { name: 'Lectura: Juan Pérez' }) as HTMLInputElement).checked).toBe(true))

    fireEvent.click(screen.getByRole('button', { name: 'Prompt de Juan Pérez' }))
    fireEvent.change(screen.getByLabelText('Instrucciones de la autorespuesta en este chat'), { target: { value: 'Sé breve.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))
    expect(runtime.saveWhatsAppChat).toHaveBeenLastCalledWith('lib', { chatId: JUAN, read: true, write: false, autoReply: false, prompt: 'Sé breve.' }, '')
    await waitFor(() => expect(screen.queryByLabelText('Instrucciones de la autorespuesta en este chat')).toBeNull())
    expect(screen.getByRole('button', { name: 'Prompt de Juan Pérez' }).textContent).toBe('Editar prompt')
  })

  it('searches the chats in the backend', async () => {
    runtime.readWhatsApp.mockResolvedValue(view())
    render(<WhatsAppSection libraryId="lib" />)
    expect(await screen.findByText('Juan Pérez')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Buscar chats de WhatsApp'), { target: { value: 'juan' } })
    await waitFor(() => expect(runtime.readWhatsApp).toHaveBeenCalledWith('lib', 'juan'))
  })

  it('explains where WhatsApp is linked on a device that cannot', async () => {
    runtime.readWhatsApp.mockResolvedValue(view({ available: false, unavailableReason: 'WhatsApp se vincula en la PC host.', enabled: false, state: 'off', chats: [] }))
    render(<WhatsAppSection libraryId="lib" />)
    expect(await screen.findByText('WhatsApp se vincula en la PC host.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Conectar' })).toBeNull()
  })
})
