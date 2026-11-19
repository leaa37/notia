// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import type { NotiaLibrary } from '../../../types/notia'
import type { FinanceMovements, FinanceOverview, FinanceProducts } from '../types/financeScreen'
import { FinanceScreen } from './FinanceScreen'

const service = vi.hoisted(() => ({
  getFinanceOverview: vi.fn(),
  getFinanceMovements: vi.fn(),
  getFinanceProducts: vi.fn(),
  getFinanceSalarySavings: vi.fn(),
}))
const composer = vi.hoisted(() => ({ requestChatComposerText: vi.fn() }))
vi.mock('../services/financeService', () => service)
vi.mock('../services/dollarQuotesService', () => ({ getDollarQuotes: () => Promise.resolve([]) }))
vi.mock('../../../services/chat/chatComposerRequests', () => composer)
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => vi.fn() }))
// El espacio de un celular: la medición no existe en happy-dom.
vi.mock('../../../hooks/useNarrowContainer', () => ({ useNarrowContainer: () => true }))

const library = { id: 'library-1' } as NotiaLibrary
const pesos = (amount: string) => ({ amount, currency: 'ARS' })

const overview = {
  review: [
    { id: 'c1', label: 'Servicio', parts: [{ text: '¿Es el pago de AMP?', strong: false }], actions: [{ label: 'Sí, es ese', prompt: 'Sí, AMP2008 es AMP Seguros.', primary: true }] },
    { id: 'c2', label: 'Categorías', parts: [{ text: '9 gastos sin categoría.', strong: false }], actions: [{ label: 'Categorizar en el chat', prompt: 'Categorizá los 9 gastos.', primary: true }] },
  ],
  salary: null,
  expenses: { totals: [pesos('1000')], count: 2, cardUnpaid: [], cardUnpaidCount: 0 },
  saved: { contributions: [], reserves: [], bought: [], cost: [], withdrawals: [], rate: null },
  categories: { rows: [], hasPrevious: false, previousMonth: '2026-08', categorizePrompt: null, uncategorizedCount: 0 },
  cards: { statements: [], totals: [] },
  services: { rows: [], pendingCount: 0 },
  latest: [],
  movementCount: 0,
  balance: { rows: [], otherIncomes: [] },
  budgets: [],
  loans: { rows: [], paid: [], pendingCount: 0 },
} as unknown as FinanceOverview

const movements = {
  month: '2026-09',
  filter: 'all',
  chips: [{ id: 'all', label: 'Todos', count: 1 }],
  summary: { expenseCount: 1, expenseTotals: [pesos('82997')], exchangeCount: 0, incomeCount: 0 },
  groups: [{
    accountId: 'a1', name: 'Mastercard ••0-3', kindLabel: 'Tarjeta', statement: null, totals: [pesos('82997')], savings: [],
    rows: [{
      id: 'm1', kind: 'expense', status: 'confirmed', description: 'MOVI STAR 09/26', purchaseDate: '2026-08-19', effectiveDate: '2026-09-04',
      countsOnStatementDue: true, accountId: 'a1', accountName: 'Mastercard ••0-3', categoryName: null, uncategorized: true,
      serviceName: 'Movistar', origin: 'Telegram', viaTelegram: true, amount: '82997', currency: 'ARS', exchange: null, flagged: false, note: null,
    }],
  }],
  changePrompts: { m1: 'Quiero cambiar MOVI STAR 09/26.' },
} as unknown as FinanceMovements

const detail = {
  id: 'p1', name: 'Yerba mate 1 kg', aliases: [], currency: 'ARS',
  best: { merchant: 'Coto', date: '2026-09-21', price: '4850' }, worst: null, changePercent: 15.5, changeMerchant: 'Coto',
  series: [{ merchant: 'Coto', points: [{ date: '2026-07-12', price: '4200' }, { date: '2026-09-21', price: '4850' }] }],
  rows: [{ merchant: 'Coto', date: '2026-09-21', price: '4850', cheapest: true, difference: null, differencePercent: null }],
  similar: null, correctPrompt: 'Corregí la yerba.',
}
const products = (selectedId: string | null) => ({
  products: [{ id: 'p1', name: 'Yerba mate 1 kg', merchantCount: 1, lastDate: '2026-09-21', fromPrice: pesos('4850'), changePercent: 15.5, firstDate: '2026-07-12' }],
  selected: selectedId === 'p1' ? detail : null,
  tickets: [],
  totalCount: 1,
}) as unknown as FinanceProducts

describe('FinanceScreen en un celular', () => {
  afterEach(() => {
    cleanup()
    vi.resetAllMocks()
  })

  it('sigue los tableros de teléfono: revisar de a una, abrir un movimiento y el detalle de un producto', async () => {
    service.getFinanceOverview.mockResolvedValue(overview)
    service.getFinanceMovements.mockResolvedValue(movements)
    service.getFinanceProducts.mockImplementation((_library: NotiaLibrary, request: { selectedId: string | null }) => Promise.resolve(products(request.selectedId)))
    render(<FinanceScreen library={library} />)
    await act(async () => undefined)

    // Para revisar: una tarjeta a la vez; la respuesta llena el chat, no se envía.
    const review = screen.getByRole('region', { name: 'Para revisar · 2' })
    expect(within(review).getByText('1 de 2')).toBeTruthy()
    fireEvent.click(within(review).getByRole('button', { name: 'Siguiente' }))
    expect(within(review).getByText('2 de 2')).toBeTruthy()
    fireEvent.click(within(review).getByRole('button', { name: 'Categorizar en el chat' }))
    expect(composer.requestChatComposerText).toHaveBeenCalledWith('Categorizá los 9 gastos.')
    // Sin pestaña Dev en el teléfono: las herramientas quedan al pie.
    expect(screen.queryByRole('tab', { name: 'Dev' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Herramientas de desarrollo' })).toBeTruthy()

    // Movimientos: la fila abre su detalle en el lugar.
    await act(async () => { fireEvent.click(screen.getByRole('tab', { name: 'Movimientos' })) })
    const row = screen.getByRole('button', { name: /MOVI STAR 09\/26/ })
    expect(row.getAttribute('aria-expanded')).toBe('false')
    fireEvent.click(row)
    expect(row.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByText('Cargado desde')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Pedir un cambio en el chat' }))
    expect(composer.requestChatComposerText).toHaveBeenLastCalledWith('Quiero cambiar MOVI STAR 09/26.')

    // Productos: el detalle reemplaza la lista y «Productos» vuelve.
    await act(async () => { fireEvent.click(screen.getByRole('tab', { name: 'Productos' })) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Yerba mate 1 kg/ })) })
    expect(service.getFinanceProducts).toHaveBeenLastCalledWith(library, expect.objectContaining({ selectedId: 'p1' }))
    expect(screen.getByRole('heading', { name: 'Yerba mate 1 kg' })).toBeTruthy()
    expect(screen.getByText('Más barato hoy')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Productos' }))
    expect(screen.getByRole('searchbox', { name: 'Buscar producto' })).toBeTruthy()
  })
})
