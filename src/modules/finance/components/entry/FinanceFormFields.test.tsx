// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import type { EntryField } from '../../types/financeEntry'
import { FinanceFormFields } from './FinanceFormFields'

const items: EntryField = {
  key: 'items', label: 'Productos', type: 'items', value: [], required: true, addLabel: 'Agregar producto',
  columns: [
    { key: 'originalDescription', label: 'Producto', type: 'text', value: '', required: true },
    { key: 'quantity', label: 'Cantidad', type: 'money', value: '1', required: false },
  ],
}

describe('FinanceFormFields', () => {
  afterEach(cleanup)

  it('agrega y quita filas con los valores por defecto de cada columna', () => {
    const onChange = vi.fn()
    const { rerender } = render(<FinanceFormFields fields={[items]} values={{}} errors={{}} disabled={false} onChange={onChange} idPrefix="t" />)
    expect(screen.getByText('Todavía no hay filas.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /Agregar producto/ }))
    expect(onChange).toHaveBeenLastCalledWith('items', [{ originalDescription: '', quantity: '1' }])
    rerender(<FinanceFormFields fields={[items]} values={{ items: [{ originalDescription: 'Leche', quantity: '2' }] }} errors={{ items: 'Revisá la fila 1.' }} disabled={false} onChange={onChange} idPrefix="t" />)
    expect((screen.getByLabelText(/^Producto/) as HTMLInputElement).value).toBe('Leche')
    expect(screen.getByRole('alert').textContent).toBe('Revisá la fila 1.')
    fireEvent.click(screen.getByRole('button', { name: 'Quitar la fila 1' }))
    expect(onChange).toHaveBeenLastCalledWith('items', [])
  })

  it('un interruptor cambia su valor y un campo de solo lectura no se edita', () => {
    const onChange = vi.fn()
    const fields: EntryField[] = [
      { key: 'active', label: 'Activo', type: 'toggle', value: true, required: false },
      { key: 'amount', label: 'Importe', type: 'money', value: '100', required: true, readOnly: true, help: 'Se corrige desde el ticket.' },
    ]
    render(<FinanceFormFields fields={fields} values={{ active: true, amount: '100' }} errors={{}} disabled={false} onChange={onChange} idPrefix="t" />)
    const toggle = screen.getByRole('button', { name: 'Activo' })
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(toggle)
    expect(onChange).toHaveBeenCalledWith('active', false)
    const amount = screen.getByLabelText('Importe') as HTMLInputElement
    expect(amount.disabled).toBe(true)
    expect(amount.getAttribute('inputmode')).toBe('decimal')
    expect(screen.getByText('Se corrige desde el ticket.')).toBeTruthy()
  })
})
