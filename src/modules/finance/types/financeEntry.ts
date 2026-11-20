// The forms Rust builds (`finance_forms.rs`) and what applying them returns
// (`finance_entry.rs`). The screen draws them with one renderer.

export type EntryValue = string | string[] | number | boolean | null
export type EntryValues = Record<string, EntryValue>

export interface EntryChoice {
  value: string
  label: string
  detail?: string
}

export interface CurrencyPick {
  /** Key of the currency in the values; empty when it is locked. */
  key: string
  value: string
  locked: boolean
}

export type EntryFieldType = 'text' | 'money' | 'integer' | 'date' | 'month' | 'select' | 'checks' | 'info'

export interface EntryField {
  key: string
  label: string
  type: EntryFieldType
  value: EntryValue
  required: boolean
  help?: string
  placeholder?: string
  options?: EntryChoice[]
  suggestions?: string[]
  currency?: CurrencyPick
  /** Changing it asks Rust for the form again, with what was typed. */
  reload?: boolean
}

export interface EntryForm {
  kind: string
  id: string | null
  title: string
  intro: string | null
  submit: string
  fields: EntryField[]
  delete: { label: string; confirm: string } | null
}

export interface EntryMenuItem {
  kind: string
  label: string
  description: string
}

export interface EntryMenuGroup {
  title: string
  items: EntryMenuItem[]
}

export interface EntryRequest {
  kind: string
  id?: string | null
  /** What the screen preselects (the service, the card, the month). */
  prefill?: EntryValues
}

export interface Undo {
  kind: string
  id: string | null
  action: string
  values: EntryValues
}

export interface ApplyResult {
  ok: boolean
  message: string
  errors: Record<string, string>
  savedId: string | null
  undo: Undo | null
  duplicate: boolean
}

export interface ManageRow {
  id: string
  kind: string
  title: string
  subtitle: string
  muted: boolean
}

export interface ManageSection {
  key: string
  title: string
  addKind: string
  addLabel: string
  empty: string
  rows: ManageRow[]
}

export interface ExportFile {
  fileName: string
  mediaType: string
  base64: string
  rows: number
}
