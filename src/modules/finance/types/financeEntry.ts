// Manual loads of Finanzas: Rust builds every form (`finance_forms.rs`) and
// applies it (`finance_entry.rs`); the screen only draws them and sends
// what the person typed.

import type { Money, MovementRow } from './financeScreen'

export interface EntryChoice {
  value: string
  label: string
}

export type EntryFieldType = 'text' | 'multiline' | 'money' | 'integer' | 'date' | 'month' | 'select' | 'toggle' | 'items'

export type EntryValue = string | boolean | EntryRow[]
export type EntryRow = Record<string, string | boolean>
export type EntryValues = Record<string, EntryValue>

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
  /** The fields of each row of an `items` field. */
  columns?: EntryField[]
  addLabel?: string
  readOnly?: boolean
}

export interface EntryForm {
  kind: string
  id: string | null
  title: string
  intro: string | null
  submit: string
  fields: EntryField[]
  delete: { label: string; confirm: string } | null
  actions: Array<{ id: string; label: string; confirm: string | null }>
}

export interface EntryMenuGroup {
  title: string
  items: Array<{ kind: string; label: string; description: string }>
}

export interface ApplyResult {
  ok: boolean
  message: string
  /** What to fix in each field, by its key. */
  errors: Record<string, string>
  savedId: string | null
  duplicate: boolean
}

/** What opens a form: a new load (with what the screen preselects) or a stored record. */
export interface EntryRequest {
  kind: string
  id?: string | null
  prefill?: Record<string, string>
}

export interface ManageRow {
  id: string
  kind: string
  title: string
  subtitle: string
  inactive: boolean
}

export interface ManageSection {
  key: string
  title: string
  addKind: string | null
  addLabel: string | null
  empty: string
  rows: ManageRow[]
}

export interface FinanceSearch {
  search: string
  filter: string
  count: number
  page: number
  hasMore: boolean
  months: Array<{ month: string; rows: MovementRow[]; expenseTotals: Money[] }>
  expenseTotals: Money[]
}

export interface ExportFile {
  fileName: string
  mediaType: string
  base64: string
  rows: number
}
