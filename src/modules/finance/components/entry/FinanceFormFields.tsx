import { Minus, Plus } from 'lucide-react'
import type { EntryField, EntryRow, EntryValue, EntryValues } from '../../types/financeEntry'

interface Props {
  fields: EntryField[]
  values: EntryValues
  errors: Record<string, string>
  disabled: boolean
  onChange: (key: string, value: EntryValue) => void
  /** Prefix of the ids, so two forms never share them. */
  idPrefix: string
}

/**
 * Draws the fields of a form Rust built. It keeps no rule of its own: what
 * is valid, the defaults and the options all come with the form.
 */
export function FinanceFormFields({ fields, values, errors, disabled, onChange, idPrefix }: Props) {
  return <div className="finance-form__fields">
    {fields.map((field) => <FieldView
      key={field.key}
      field={field}
      value={values[field.key] ?? field.value}
      error={errors[field.key]}
      disabled={disabled}
      id={`${idPrefix}-${field.key}`}
      onChange={(value) => onChange(field.key, value)}
    />)}
  </div>
}

interface FieldProps {
  field: EntryField
  value: EntryValue
  error?: string
  disabled: boolean
  id: string
  onChange: (value: EntryValue) => void
}

function FieldView({ field, value, error, disabled, id, onChange }: FieldProps) {
  const describedBy = [field.help ? `${id}-help` : null, error ? `${id}-error` : null].filter(Boolean).join(' ') || undefined
  const locked = disabled || Boolean(field.readOnly)
  if (field.type === 'items') {
    return <ItemsField field={field} rows={Array.isArray(value) ? value : []} error={error} disabled={locked} id={id} onChange={onChange} />
  }
  if (field.type === 'toggle') {
    const on = value === true
    return <div className="finance-form__field finance-form__field--toggle">
      <button type="button" id={id} className="finance-switch" aria-pressed={on} disabled={locked} aria-describedby={describedBy} onClick={() => onChange(!on)}>
        <span className="finance-switch__track" aria-hidden="true"><span className="finance-switch__knob" /></span>
        {field.label}
      </button>
      {field.help && <span id={`${id}-help`} className="finance-form__help">{field.help}</span>}
      {error && <span id={`${id}-error`} className="finance-form__error" role="alert">{error}</span>}
    </div>
  }
  const text = typeof value === 'string' ? value : ''
  const common = {
    id,
    disabled: locked,
    'aria-invalid': error ? true : undefined,
    'aria-describedby': describedBy,
    'aria-required': field.required || undefined,
  }
  let control
  if (field.type === 'select') {
    control = <select {...common} value={text} onChange={(event) => onChange(event.target.value)}>
      {!(field.options ?? []).some((option) => option.value === '') && <option value="" disabled>Elegí…</option>}
      {(field.options ?? []).map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
    </select>
  } else if (field.type === 'multiline') {
    control = <textarea {...common} rows={3} value={text} placeholder={field.placeholder} onChange={(event) => onChange(event.target.value)} />
  } else {
    const inputType = field.type === 'date' ? 'date' : field.type === 'month' ? 'month' : 'text'
    const inputMode = field.type === 'money' ? 'decimal' : field.type === 'integer' ? 'numeric' : undefined
    const listId = field.suggestions && field.suggestions.length > 0 ? `${id}-suggestions` : undefined
    control = <>
      <input
        {...common}
        type={inputType}
        inputMode={inputMode}
        list={listId}
        autoComplete="off"
        value={text}
        placeholder={field.placeholder ?? (field.type === 'month' ? 'AAAA-MM' : undefined)}
        onChange={(event) => onChange(event.target.value)}
      />
      {listId && <datalist id={listId}>{field.suggestions?.map((suggestion) => <option key={suggestion} value={suggestion} />)}</datalist>}
    </>
  }
  return <div className={`finance-form__field finance-form__field--${field.type}`}>
    <label htmlFor={id} className="finance-form__label">
      {field.label}{field.required && !field.readOnly && <span className="finance-form__required" aria-hidden="true"> *</span>}
    </label>
    {control}
    {field.help && <span id={`${id}-help`} className="finance-form__help">{field.help}</span>}
    {error && <span id={`${id}-error`} className="finance-form__error" role="alert">{error}</span>}
  </div>
}

function emptyRow(columns: EntryField[]): EntryRow {
  const row: EntryRow = {}
  for (const column of columns) {
    row[column.key] = typeof column.value === 'boolean' ? column.value : typeof column.value === 'string' ? column.value : ''
  }
  return row
}

function ItemsField({ field, rows, error, disabled, id, onChange }: { field: EntryField; rows: EntryRow[]; error?: string; disabled: boolean; id: string; onChange: (value: EntryValue) => void }) {
  const columns = field.columns ?? []
  const change = (index: number, key: string, value: EntryValue) => {
    onChange(rows.map((row, position) => position === index ? { ...row, [key]: value as string | boolean } : row))
  }
  return <fieldset className="finance-form__items" aria-describedby={error ? `${id}-error` : undefined}>
    <legend className="finance-form__label">{field.label}{field.required && <span className="finance-form__required" aria-hidden="true"> *</span>}</legend>
    {field.help && <span className="finance-form__help">{field.help}</span>}
    {rows.length === 0 && <p className="finance-form__help">Todavía no hay filas.</p>}
    {rows.map((row, index) => <div key={index} className="finance-form__item" role="group" aria-label={`Fila ${index + 1}`}>
      <span className="finance-form__item-number" aria-hidden="true">{index + 1}</span>
      <div className="finance-form__item-fields">
        {columns.map((column) => <FieldView
          key={column.key}
          field={column}
          value={row[column.key] ?? column.value}
          disabled={disabled}
          id={`${id}-${index}-${column.key}`}
          onChange={(value) => change(index, column.key, value)}
        />)}
      </div>
      <button type="button" className="finance-icon-button finance-form__item-remove" aria-label={`Quitar la fila ${index + 1}`} disabled={disabled} onClick={() => onChange(rows.filter((_, position) => position !== index))}>
        <Minus size={16} aria-hidden="true" />
      </button>
    </div>)}
    <button type="button" className="finance-button finance-button--quiet-outline finance-button--small" disabled={disabled} onClick={() => onChange([...rows, emptyRow(columns)])}>
      <Plus size={14} aria-hidden="true" />{field.addLabel ?? 'Agregar'}
    </button>
    {error && <span id={`${id}-error`} className="finance-form__error" role="alert">{error}</span>}
  </fieldset>
}
