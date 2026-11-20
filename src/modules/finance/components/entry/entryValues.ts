import type { EntryField, EntryValues } from '../../types/financeEntry'

/** The values a form starts with: each field's, and each currency's. */
export function initialValues(fields: EntryField[]): EntryValues {
  const values: EntryValues = {}
  for (const field of fields) {
    if (field.type === 'info') continue
    values[field.key] = field.value
    if (field.currency && !field.currency.locked && field.currency.key) values[field.currency.key] = field.currency.value
  }
  return values
}
