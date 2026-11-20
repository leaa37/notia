// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import type { NotiaLibrary } from '../../../types/notia'
import type { EntryForm } from '../types/financeEntry'
import { manage, movements, overview, payments, savings } from './financeTestData'
import { FinanceScreen } from './FinanceScreen'

const service = vi.hoisted(() => ({
  getFinanceOverview: vi.fn(),
  getFinancePayments: vi.fn(),
  getFinanceMovements: vi.fn(),
  searchFinanceHistory: vi.fn(),
  getFinanceSavings: vi.fn(),
  listFinanceDevTables: vi.fn(() => Promise.resolve([])),
}))
const entry = vi.hoisted(() => ({
  getEntryMenu: vi.fn(),
  getEntryForm: vi.fn(),
  applyEntry: vi.fn(),
  getManage: vi.fn(),
  exportFinance: vi.fn(),
  newRequestId: vi.fn(() => 'pedido-1'),
}))
const events = vi.hoisted(() => ({ notifyFinanceDataChanged: vi.fn(), subscribeToFinanceDataChanges: vi.fn(() => () => undefined) }))
vi.mock('../services/financeService', () => service)
vi.mock('../services/financeEntryService', () => entry)
vi.mock('../services/financeDataEvents', () => events)
vi.mock('../../../services/chat/chatComposerRequests', () => ({ requestChatComposerText: vi.fn() }))
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => vi.fn() }))
// The desktop: happy-dom does not measure.
vi.mock('../../../hooks/useNarrowContainer', () => ({ useNarrowContainer: () => false }))

const library = { id: 'library-1' } as NotiaLibrary

function form(partial: Partial<EntryForm>): EntryForm {
  return { kind: 'expense', id: null, title: 'Nuevo gasto', intro: null, submit: 'Guardar', fields: [], delete: null, ...partial }
}

const expenseForm = form({
  fields: [
    { key: 'name', label: '¿En qué?', type: 'text', value: '', required: true },
    { key: 'amount', label: 'Importe', type: 'money', value: '', required: true, currency: { key: 'currency', value: 'ARS', locked: false } },
    { key: 'categoryId', label: 'Categoría', type: 'select', value: '', required: true, options: [{ value: 'category-transport', label: 'Transporte' }] },
  ],
})

async function flush() {
  await act(async () => { await Promise.resolve() })
  await act(async () => { await Promise.resolve() })
}

beforeEach(() => {
  service.getFinanceOverview.mockResolvedValue(overview)
  service.getFinancePayments.mockResolvedValue(payments)
  service.getFinanceMovements.mockResolvedValue(movements)
  service.getFinanceSavings.mockResolvedValue(savings)
  entry.getManage.mockResolvedValue(manage)
  entry.getEntryMenu.mockResolvedValue([{ title: 'Del día a día', items: [{ kind: 'expense', label: 'Gasto', description: 'Nombre, importe y categoría' }] }])
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Finanzas', () => {
  it('has Resumen, Pagos, Movimientos, Ahorro and Administrar, and nothing of tickets or products', async () => {
    render(<FinanceScreen library={library} />)
    await flush()
    const tabs = screen.getAllByRole('tab').map((tab) => tab.textContent)
    expect(tabs).toEqual(['Resumen', 'Pagos', 'Movimientos', 'Ahorro', 'Administrar', 'Dev'])
    expect(screen.queryByText(/ticket|producto/i)).toBeNull()
  })

  it('tells what is left, what is missing to pay and how the budgets go', async () => {
    render(<FinanceScreen library={library} />)
    await flush()
    expect(screen.getByText('Te queda')).toBeTruthy()
    expect(screen.getByText('$ 1.000.000')).toBeTruthy()
    expect(screen.getByText('Pagaste 1 de 3')).toBeTruthy()
    // A service not paid yet has no amount: what is missing is the cards' and a note.
    expect(screen.getByText('$ 90.000 + 1 servicio')).toBeTruthy()
    expect(screen.getByText(/Te pasaste \$ 30\.000 del presupuesto/)).toBeTruthy()
    expect(screen.getByText(/que el mes pasado/)).toBeTruthy()
    // Paying a service from the overview opens its form with the service.
    entry.getEntryForm.mockResolvedValue(form({ kind: 'service-payment', title: 'Pagar un servicio' }))
    const internet = screen.getByText('Internet').closest('li') as HTMLElement
    fireEvent.click(within(internet).getByRole('button', { name: 'Pagar' }))
    await flush()
    expect(entry.getEntryForm).toHaveBeenCalledWith(library, { kind: 'service-payment', prefill: { serviceId: 's1', month: '2026-11' } })
  })

  it('pays a card installment with one touch and can undo it', async () => {
    entry.applyEntry.mockResolvedValueOnce({ ok: true, message: 'Cuota de Visa pagada: $ 60.000.', errors: {}, savedId: 'm9', undo: { kind: 'movements', id: null, action: 'delete', values: { ids: ['m9'] } }, duplicate: false })
    render(<FinanceScreen library={library} />)
    await flush()
    fireEvent.click(screen.getByRole('tab', { name: 'Pagos' }))
    await flush()
    const fridge = screen.getByText('Heladera', { selector: '.finance-pay-row__name' }).closest('li') as HTMLElement
    fireEvent.click(within(fridge).getByRole('button', { name: 'Pagar' }))
    await flush()
    expect(entry.applyEntry).toHaveBeenCalledWith(library, { kind: 'card-payment', id: null, values: { cardId: 'c1', installments: ['p1:3'] }, requestId: 'pedido-1' })
    expect(screen.getByText('Cuota de Visa pagada: $ 60.000.')).toBeTruthy()
    entry.applyEntry.mockResolvedValueOnce({ ok: true, message: '1 movimientos borrados.', errors: {}, savedId: 'm9', undo: null, duplicate: false })
    fireEvent.click(screen.getByRole('button', { name: 'Deshacer' }))
    await flush()
    expect(entry.applyEntry).toHaveBeenLastCalledWith(library, { kind: 'movements', id: null, action: 'delete', values: { ids: ['m9'] }, requestId: 'pedido-1' })
    expect(screen.getByText('Listo, lo deshice.')).toBeTruthy()
    expect(screen.getByText(/atrasada/)).toBeTruthy()
    expect(screen.getByText('$ 90.000 + 1 servicio y 1 cuota de préstamo')).toBeTruthy()
    expect(screen.getByRole('button', { name: /Pagar las 2 cuotas/ })).toBeTruthy()
  })

  it('loads by hand, names what to fix by field and keeps what was typed', async () => {
    entry.getEntryForm.mockResolvedValue(expenseForm)
    entry.applyEntry.mockResolvedValueOnce({ ok: false, message: 'Elegí la categoría.', errors: { categoryId: 'Elegí la categoría.' }, savedId: null, undo: null, duplicate: false })
    render(<FinanceScreen library={library} />)
    await flush()
    fireEvent.click(screen.getByRole('button', { name: 'Cargar' }))
    await flush()
    fireEvent.click(screen.getByRole('button', { name: /^Gasto/ }))
    await flush()
    const dialog = screen.getByRole('dialog', { name: 'Nuevo gasto' })
    fireEvent.change(within(dialog).getByLabelText(/¿En qué\?/), { target: { value: 'Nafta' } })
    fireEvent.change(within(dialog).getByLabelText(/Importe/), { target: { value: '45.000' } })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Guardar' }))
    await flush()
    expect(entry.applyEntry).toHaveBeenCalledWith(library, { kind: 'expense', id: null, action: 'save', values: { name: 'Nafta', amount: '45.000', currency: 'ARS', categoryId: '' }, requestId: 'pedido-1' })
    expect(within(dialog).getByText('Elegí la categoría.')).toBeTruthy()
    // Closing with something typed asks first: nothing is lost by accident.
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
    expect(within(dialog).getByText('¿Descartar lo que cargaste?')).toBeTruthy()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Seguir cargando' }))
    expect((within(dialog).getByLabelText(/¿En qué\?/) as HTMLInputElement).value).toBe('Nafta')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
    fireEvent.click(within(dialog).getByRole('button', { name: 'Descartar' }))
    expect(screen.queryByRole('dialog', { name: 'Nuevo gasto' })).toBeNull()
  })

  it('asks Rust for the form again when a field with reload changes', async () => {
    const first = form({
      kind: 'service-payment',
      title: 'Pagar un servicio',
      fields: [
        { key: 'serviceId', label: 'Servicio', type: 'select', value: 's1', required: true, reload: true, options: [{ value: 's1', label: 'Internet' }, { value: 's2', label: 'Luz' }] },
        { key: 'amount', label: 'Lo que pagaste', type: 'money', value: '25000', required: true, currency: { key: '', value: 'ARS', locked: true } },
      ],
    })
    const second = { ...first, fields: [first.fields[0], { ...first.fields[1], value: '18000' }].map((field, index) => (index === 0 ? { ...field, value: 's2' } : field)) }
    entry.getEntryForm.mockResolvedValueOnce(first).mockResolvedValueOnce(second)
    render(<FinanceScreen library={library} />)
    await flush()
    fireEvent.click(screen.getByRole('tab', { name: 'Pagos' }))
    await flush()
    const internet = screen.getByText('Internet', { selector: '.finance-pay-row__name' }).closest('li') as HTMLElement
    fireEvent.click(within(internet).getByRole('button', { name: 'Pagar' }))
    await flush()
    const dialog = screen.getByRole('dialog', { name: 'Pagar un servicio' })
    fireEvent.change(within(dialog).getByLabelText(/Servicio/), { target: { value: 's2' } })
    await flush()
    expect(entry.getEntryForm).toHaveBeenLastCalledWith(library, { kind: 'service-payment', id: null, prefill: { serviceId: 's2', amount: '25000', _changed: 'serviceId' } })
    expect((within(dialog).getByLabelText(/Lo que pagaste/) as HTMLInputElement).value).toBe('18000')
  })

  it('edits or deletes what is set up from Administrar', async () => {
    entry.getEntryForm.mockResolvedValue(form({ kind: 'category', id: 'category-supermarket', title: 'Categoría', fields: [{ key: 'name', label: 'Nombre', type: 'text', value: 'Supermercado', required: true }], delete: { label: 'Borrar categoría', confirm: '¿Borrar la categoría?' } }))
    entry.applyEntry.mockResolvedValueOnce({ ok: true, message: 'Categoría borrada.', errors: {}, savedId: 'category-supermarket', undo: null, duplicate: false })
    render(<FinanceScreen library={library} />)
    await flush()
    fireEvent.click(screen.getByRole('tab', { name: 'Administrar' }))
    await flush()
    expect(screen.queryByText(/Desactivar/)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /Supermercado/ }))
    await flush()
    const dialog = screen.getByRole('dialog', { name: 'Categoría' })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Borrar categoría' }))
    fireEvent.click(within(within(dialog).getByRole('alertdialog')).getByRole('button', { name: 'Borrar categoría' }))
    await flush()
    expect(entry.applyEntry).toHaveBeenCalledWith(library, { kind: 'category', id: 'category-supermarket', action: 'delete', values: undefined, requestId: 'pedido-1' })
    expect(screen.getByText('Categoría borrada.')).toBeTruthy()
    expect(events.notifyFinanceDataChanged).toHaveBeenCalled()
  })

  it('shows savings without the salary', async () => {
    render(<FinanceScreen library={library} />)
    await flush()
    fireEvent.click(screen.getByRole('tab', { name: 'Ahorro' }))
    await flush()
    expect(screen.getByText('Ahorrado en pesos')).toBeTruthy()
    expect(screen.getByText('30 % de $ 1.000.000')).toBeTruthy()
    expect(screen.queryByText(/sueldo/i)).toBeNull()
  })
})
