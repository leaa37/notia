// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { EntryField } from '../../types/financeEntry'
import { FinanceFormFields } from './FinanceFormFields'
import { initialValues } from './entryValues'

afterEach(cleanup)

const amount: EntryField = { key: 'amount', label: 'Importe', type: 'money', value: '1500', required: true, currency: { key: 'currency', value: 'ARS', locked: false } }
const installments: EntryField = {
  key: 'installments',
  label: 'Cuotas a pagar',
  type: 'checks',
  value: ['p:1', 'p:2'],
  required: true,
  options: [{ value: 'p:1', label: 'Heladera · cuota 1/12', detail: '$ 60.000' }, { value: 'p:2', label: 'Zapatillas', detail: '$ 30.000' }],
}

describe('FinanceFormFields', () => {
  it('starts with each value and the currency of each amount', () => {
    expect(initialValues([amount, installments, { key: 'what', label: 'Pago de', type: 'info', value: 'Cuota 1/3', required: false }])).toEqual({
      amount: '1500',
      currency: 'ARS',
      installments: ['p:1', 'p:2'],
    })
  })

  it('switches the currency beside the amount and keeps it under its own key', () => {
    const onChange = vi.fn()
    render(<FinanceFormFields fields={[amount]} values={{ amount: '1500', currency: 'ARS' }} errors={{}} disabled={false} onChange={onChange} idPrefix="t" />)
    expect(screen.getByRole('radio', { name: 'Pesos' }).getAttribute('aria-checked')).toBe('true')
    fireEvent.click(screen.getByRole('radio', { name: 'Dólares' }))
    expect(onChange).toHaveBeenCalledWith('currency', 'USD', amount)
  })

  it('shows a locked currency as text', () => {
    render(<FinanceFormFields fields={[{ ...amount, currency: { key: '', value: 'USD', locked: true } }]} values={{}} errors={{}} disabled={false} onChange={vi.fn()} idPrefix="t" />)
    expect(screen.getByText('US$')).toBeTruthy()
    expect(screen.queryByRole('radio')).toBeNull()
  })

  it('chooses installments one by one with big touch targets', () => {
    const onChange = vi.fn()
    render(<FinanceFormFields fields={[installments]} values={{ installments: ['p:1', 'p:2'] }} errors={{}} disabled={false} onChange={onChange} idPrefix="t" />)
    const first = screen.getByRole('checkbox', { name: /Heladera/ })
    expect(first.getAttribute('aria-checked')).toBe('true')
    fireEvent.click(first)
    expect(onChange).toHaveBeenCalledWith('installments', ['p:2'], installments)
  })

  it('draws two short options without a label as buttons', () => {
    const onChange = vi.fn()
    const type: EntryField = { key: 'type', label: '', type: 'select', value: 'deposit', required: true, options: [{ value: 'deposit', label: 'Guardar' }, { value: 'withdrawal', label: 'Sacar' }] }
    render(<FinanceFormFields fields={[type]} values={{ type: 'deposit' }} errors={{}} disabled={false} onChange={onChange} idPrefix="t" />)
    fireEvent.click(screen.getByRole('radio', { name: 'Sacar' }))
    expect(onChange).toHaveBeenCalledWith('type', 'withdrawal', type)
  })

  it('names the error of a field beside it', () => {
    render(<FinanceFormFields fields={[amount]} values={{ amount: 'abc' }} errors={{ amount: 'El importe tiene que ser un número.' }} disabled={false} onChange={vi.fn()} idPrefix="t" />)
    expect(screen.getByRole('alert').textContent).toBe('El importe tiene que ser un número.')
    expect(screen.getByLabelText(/Importe/).getAttribute('aria-invalid')).toBe('true')
  })
})
