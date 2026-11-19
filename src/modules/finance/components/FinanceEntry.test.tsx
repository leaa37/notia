// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import type { NotiaLibrary } from '../../../types/notia'
import type { EntryForm } from '../types/financeEntry'
import type { FinanceMovements, FinanceOverview } from '../types/financeScreen'
import { FinanceScreen } from './FinanceScreen'

const service = vi.hoisted(() => ({
  getFinanceOverview: vi.fn(),
  getFinanceMovements: vi.fn(),
  getFinanceProducts: vi.fn(),
  getFinanceSalarySavings: vi.fn(),
}))
const entry = vi.hoisted(() => ({
  getEntryMenu: vi.fn(),
  getEntryForm: vi.fn(),
  applyEntry: vi.fn(),
  getManage: vi.fn(),
  resolveReview: vi.fn(),
  searchHistory: vi.fn(),
  exportFinance: vi.fn(),
  newRequestId: vi.fn(() => 'pedido-1'),
}))
const events = vi.hoisted(() => ({ notifyFinanceDataChanged: vi.fn(), subscribeToFinanceDataChanges: vi.fn(() => () => undefined) }))
vi.mock('../services/financeService', () => service)
vi.mock('../services/financeEntryService', () => entry)
vi.mock('../services/financeDataEvents', () => events)
vi.mock('../services/dollarQuotesService', () => ({ getDollarQuotes: () => Promise.resolve([]) }))
vi.mock('../../../services/chat/chatComposerRequests', () => ({ requestChatComposerText: vi.fn() }))
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => vi.fn() }))
// El escritorio: la medición no existe en happy-dom.
vi.mock('../../../hooks/useNarrowContainer', () => ({ useNarrowContainer: () => false }))

const library = { id: 'library-1' } as NotiaLibrary
const pesos = (amount: string) => ({ amount, currency: 'ARS' })

const overview = {
  review: [{ id: 'caso-1', label: 'Producto', parts: [{ text: '¿Es la misma leche?', strong: false }], actions: [
    { label: 'Sí, unirlos', prompt: 'Sí.', primary: true, optionId: 'merge' },
    { label: 'Responder en el chat', prompt: 'Sobre la duda…', primary: false, optionId: null },
  ] }],
  salary: null,
  balance: { rows: [{ currency: 'ARS', income: '5000.00', salary: '0.00', otherIncome: '5000.00', expenses: '1500.50', savings: '0.00', left: '3499.50' }], otherIncomes: [] },
  budgets: [{ id: 'b1', categoryId: 'c1', categoryName: 'Supermercado', currency: 'ARS', budget: '1000.00', spent: '1500.50', remaining: '-500.50', percent: 150.1, status: 'over' }],
  loans: { rows: [{ id: 'auto', name: 'Auto', lender: null, currency: 'ARS', number: 1, expected: '1000.00', paid: null, status: 'pending', paidCount: 0, installmentCount: 3, remaining: '3000.00', nextDueDate: '2026-11-19' }], paid: [], pendingCount: 1 },
  expenses: { totals: [pesos('1500.50')], count: 1, cardUnpaid: [], cardUnpaidCount: 0 },
  saved: { contributions: [], reserves: [], bought: [], cost: [], withdrawals: [], rate: null },
  categories: { rows: [], hasPrevious: false, previousMonth: '2026-10', categorizePrompt: null, uncategorizedCount: 0 },
  cards: { statements: [], totals: [], installments: { pendingCount: 0, remaining: [], nextDueDate: null } },
  services: { rows: [{ serviceId: 's1', name: 'Internet', provider: null, currency: 'ARS', expected: '15000.00', paid: null, status: 'pending', note: null, history: [] }], pendingCount: 1, historyPeriods: [] },
  largest: [],
  latest: [],
  movementCount: 1,
} as unknown as FinanceOverview

const movements = {
  month: '2026-11', filter: 'all', chips: [{ id: 'all', label: 'Todos', count: 1 }],
  summary: { expenseCount: 1, expenseTotals: [pesos('1500.50')], exchangeCount: 0, incomeCount: 0 },
  groups: [{ accountId: 'a1', badge: 'GA', name: 'Galicia', kindLabel: 'Banco', statement: null, totals: [pesos('1500.50')], savings: [], rows: [{
    id: 'm1', kind: 'expense', status: 'confirmed', description: 'Coto', purchaseDate: '2026-11-18', effectiveDate: '2026-11-18', countsOnStatementDue: false,
    accountId: 'a1', accountName: 'Galicia', categoryName: 'Supermercado', uncategorized: false, serviceName: null, origin: 'Carga manual', viaTelegram: false,
    amount: '1500.50', currency: 'ARS', exchange: null, flagged: false, note: null,
  }] }],
  changePrompts: {},
} as unknown as FinanceMovements

const expenseForm: EntryForm = {
  kind: 'expense', id: null, title: 'Nuevo gasto', intro: 'Con tarjeta queda a pagar.', submit: 'Guardar gasto', delete: null, actions: [],
  fields: [
    { key: 'amount', label: 'Importe', type: 'money', value: '', required: true },
    { key: 'accountId', label: 'Cuenta', type: 'select', value: '', required: true, options: [{ value: 'a1', label: 'Galicia · banco · ARS' }] },
    { key: 'effectiveDate', label: 'Fecha', type: 'date', value: '2026-11-18', required: true },
    { key: 'description', label: 'Descripción', type: 'text', value: '', required: true },
  ],
}

function setUp() {
  service.getFinanceOverview.mockResolvedValue(overview)
  service.getFinanceMovements.mockResolvedValue(movements)
  entry.getEntryMenu.mockResolvedValue([{ title: 'Del día a día', items: [{ kind: 'expense', label: 'Gasto', description: 'Algo que pagaste.' }] }])
  entry.getEntryForm.mockResolvedValue(expenseForm)
}

describe('Finanzas cargado a mano', () => {
  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  it('carga un gasto desde «Cargar»: muestra lo que falta y avisa cuando quedó guardado', async () => {
    setUp()
    entry.applyEntry
      .mockResolvedValueOnce({ ok: false, message: 'Completá «Descripción».', errors: { description: 'Completá «Descripción».' }, savedId: null, duplicate: false })
      .mockResolvedValueOnce({ ok: true, message: 'Gasto guardado: $ 1.500,50.', errors: {}, savedId: 'm2', duplicate: false })
    render(<FinanceScreen library={library} />)
    await act(async () => undefined)

    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cargar' })) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Gasto/ })) })
    expect(entry.getEntryForm).toHaveBeenCalledWith(library, { kind: 'expense' })
    const dialog = screen.getByRole('dialog', { name: 'Nuevo gasto' })
    expect(within(dialog).getByText('Con tarjeta queda a pagar.')).toBeTruthy()
    fireEvent.change(within(dialog).getByLabelText(/Importe/), { target: { value: '1.500,50' } })
    fireEvent.change(within(dialog).getByLabelText(/Cuenta/), { target: { value: 'a1' } })
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: 'Guardar gasto' })) })
    expect(entry.applyEntry).toHaveBeenCalledWith(library, {
      kind: 'expense', id: null, action: 'save', requestId: 'pedido-1',
      values: { amount: '1.500,50', accountId: 'a1', effectiveDate: '2026-11-18', description: '' },
    })
    // Lo que falta queda junto a su campo; nada se cerró.
    const description = within(dialog).getByLabelText(/Descripción/)
    expect(description.getAttribute('aria-invalid')).toBe('true')
    expect(within(dialog).getAllByText('Completá «Descripción».').length).toBeGreaterThan(0)

    fireEvent.change(description, { target: { value: 'Coto' } })
    expect(description.getAttribute('aria-invalid')).toBeNull()
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: 'Guardar gasto' })) })
    expect(screen.queryByRole('dialog', { name: 'Nuevo gasto' })).toBeNull()
    expect(screen.getByRole('status').textContent).toContain('Gasto guardado')
    expect(events.notifyFinanceDataChanged).toHaveBeenCalled()
  })

  it('paga un servicio y un préstamo desde el resumen, y resuelve una duda sin el chat', async () => {
    setUp()
    entry.resolveReview.mockResolvedValue({ ok: true, message: 'Listo, quedó resuelto.', errors: {}, savedId: null, duplicate: false })
    render(<FinanceScreen library={library} />)
    await act(async () => undefined)

    expect(screen.getByRole('region', { name: /Saldo de/ }).textContent).toContain('$ 3.499,50')
    expect(screen.getByRole('region', { name: 'Presupuestos' }).textContent).toContain('Te pasaste')

    const services = screen.getByRole('region', { name: /Servicios de/ })
    await act(async () => { fireEvent.click(within(services).getByRole('button', { name: 'Pagar' })) })
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'service-payment', prefill: { serviceId: 's1' } })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cancelar' })) })

    const loans = screen.getByRole('region', { name: /Préstamos pagados/ })
    await act(async () => { fireEvent.click(within(loans).getByRole('button', { name: 'Pagar' })) })
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'loan-payment', prefill: { loanId: 'auto' } })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cancelar' })) })

    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Sí, unirlos' })) })
    expect(entry.resolveReview).toHaveBeenCalledWith(library, 'caso-1', 'merge')
  })

  it('edita un movimiento desde su detalle y lo administrable desde «Administrar»', async () => {
    setUp()
    entry.getManage.mockResolvedValue([{ key: 'accounts', title: 'Cuentas', addKind: 'account', addLabel: 'Nueva cuenta', empty: 'Sin cuentas.', rows: [{ id: 'a1', kind: 'account', title: 'Galicia', subtitle: 'Cuenta bancaria · ARS', inactive: false }] }])
    render(<FinanceScreen library={library} />)
    await act(async () => undefined)

    await act(async () => { fireEvent.click(screen.getByRole('tab', { name: 'Movimientos' })) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Editar' })) })
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'movement', id: 'm1' })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cancelar' })) })

    await act(async () => { fireEvent.click(screen.getByRole('tab', { name: 'Administrar' })) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Galicia/ })) })
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'account', id: 'a1' })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Cancelar' })) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Nueva cuenta/ })) })
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'account' })
  })
})
