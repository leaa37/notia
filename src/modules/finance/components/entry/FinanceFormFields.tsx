import { Check } from 'lucide-react'
import type { EntryField, EntryValue, EntryValues } from '../../types/financeEntry'

interface Props {
  fields: EntryField[]
  values: EntryValues
  errors: Record<string, string>
  disabled: boolean
  onChange: (key: string, value: EntryValue, field: EntryField) => void
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
      values={values}
      error={errors[field.key] ?? (field.currency?.key ? errors[field.currency.key] : undefined)}
      disabled={disabled}
      id={`${idPrefix}-${field.key}`}
      onChange={(key, value) => onChange(key, value, field)}
    />)}
  </div>
}

interface FieldProps {
  field: EntryField
  values: EntryValues
  error?: string
  disabled: boolean
  id: string
  onChange: (key: string, value: EntryValue) => void
}

const CURRENCIES = [{ value: 'ARS', label: '$' }, { value: 'USD', label: 'US$' }]

function FieldView({ field, values, error, disabled, id, onChange }: FieldProps) {
  const value = values[field.key] ?? field.value
  const describedBy = [field.help ? `${id}-help` : null, error ? `${id}-error` : null].filter(Boolean).join(' ') || undefined
  const notes = <>
    {field.help && <span id={`${id}-help`} className="finance-form__help">{field.help}</span>}
    {error && <span id={`${id}-error`} className="finance-form__error" role="alert">{error}</span>}
  </>
  if (field.type === 'info') {
    return <div className="finance-form__field finance-form__field--info">
      <span className="finance-form__label">{field.label}</span>
      <p className="finance-form__info">{typeof field.value === 'string' ? field.value : ''}</p>
    </div>
  }
  if (field.type === 'checks') {
    const chosen = Array.isArray(value) ? value : []
    const toggle = (option: string) => onChange(field.key, chosen.includes(option) ? chosen.filter((item) => item !== option) : [...chosen, option])
    return <fieldset className="finance-form__field finance-form__checks" aria-describedby={describedBy}>
      <legend className="finance-form__label">{field.label}</legend>
      {(field.options ?? []).map((option) => {
        const on = chosen.includes(option.value)
        return <button key={option.value} type="button" role="checkbox" aria-checked={on} className="finance-check-row" disabled={disabled} onClick={() => toggle(option.value)}>
          <span className="finance-check-row__box" aria-hidden="true">{on && <Check size={14} strokeWidth={3} />}</span>
          <span className="finance-check-row__label">{option.label}</span>
          {option.detail && <span className="finance-check-row__detail">{option.detail}</span>}
        </button>
      })}
      {notes}
    </fieldset>
  }
  const text = typeof value === 'string' ? value : typeof value === 'number' ? String(value) : ''
  const common = {
    id,
    disabled,
    'aria-invalid': error ? true : undefined,
    'aria-describedby': describedBy,
    'aria-required': field.required || undefined,
  }
  let control
  if (field.type === 'select') {
    const options = field.options ?? []
    // Two or three short options read better as buttons.
    if (options.length > 0 && options.length <= 3 && options.every((option) => option.label.length <= 18) && !field.label) {
      control = <div className="finance-segmented" role="radiogroup" aria-label={field.label || 'Tipo'}>
        {options.map((option) => <button key={option.value} type="button" role="radio" aria-checked={text === option.value} disabled={disabled} onClick={() => onChange(field.key, option.value)}>{option.label}</button>)}
      </div>
    } else {
      control = <select {...common} value={text} onChange={(event) => onChange(field.key, event.target.value)}>
        {!options.some((option) => option.value === '') && <option value="" disabled>Elegí…</option>}
        {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
      </select>
    }
  } else {
    const inputType = field.type === 'date' ? 'date' : field.type === 'month' ? 'month' : 'text'
    const inputMode = field.type === 'money' ? 'decimal' : field.type === 'integer' ? 'numeric' : undefined
    const listId = field.suggestions && field.suggestions.length > 0 ? `${id}-suggestions` : undefined
    const input = <input
      {...common}
      type={inputType}
      inputMode={inputMode}
      list={listId}
      autoComplete="off"
      enterKeyHint="next"
      value={text}
      placeholder={field.placeholder ?? (field.type === 'month' ? 'AAAA-MM' : field.type === 'money' ? '0' : undefined)}
      onChange={(event) => onChange(field.key, event.target.value)}
    />
    const pick = field.currency
    control = <>
      {pick ? <div className="finance-money">
        {input}
        {pick.locked ? <span className="finance-money__currency">{pick.value === 'USD' ? 'US$' : '$'}</span> : <div className="finance-money__switch" role="radiogroup" aria-label="Moneda">
          {CURRENCIES.map((currency) => {
            const current = (values[pick.key] as string | undefined) ?? pick.value
            return <button key={currency.value} type="button" role="radio" aria-checked={current === currency.value} aria-label={currency.value === 'USD' ? 'Dólares' : 'Pesos'} disabled={disabled} onClick={() => onChange(pick.key, currency.value)}>{currency.label}</button>
          })}
        </div>}
      </div> : input}
      {listId && <datalist id={listId}>{field.suggestions?.map((suggestion) => <option key={suggestion} value={suggestion} />)}</datalist>}
    </>
  }
  return <div className={`finance-form__field finance-form__field--${field.type}`}>
    {field.label && <label htmlFor={id} className="finance-form__label">
      {field.label}{field.required && <span className="finance-form__required" aria-hidden="true"> *</span>}
    </label>}
    {control}
    {notes}
  </div>
}
