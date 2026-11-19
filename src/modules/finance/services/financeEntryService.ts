import { callBackend } from '../../../services/transport'
import type { NotiaLibrary } from '../../../types/notia'
import { financeContext } from '../types/financeTypes'
import type { ApplyResult, EntryForm, EntryMenuGroup, EntryRequest, EntryValues, ExportFile, FinanceSearch, ManageSection } from '../types/financeEntry'

// Manual loads: Rust builds the forms and decides what is saved; these calls
// only carry the person's intent and what they typed.

export function getEntryMenu(library: NotiaLibrary): Promise<EntryMenuGroup[]> {
  return callBackend<EntryMenuGroup[]>('finance_entry_menu', { context: financeContext(library) })
}

export function getEntryForm(library: NotiaLibrary, request: EntryRequest): Promise<EntryForm> {
  return callBackend<EntryForm>('finance_form', {
    payload: { context: financeContext(library), kind: request.kind, id: request.id ?? null, prefill: request.prefill ?? {} },
  })
}

export interface ApplyRequest {
  kind: string
  id: string | null
  /** `save` (the default), `delete` or the id of one of the form's actions. */
  action?: string
  values?: EntryValues
  requestId: string
}

export function applyEntry(library: NotiaLibrary, request: ApplyRequest): Promise<ApplyResult> {
  return callBackend<ApplyResult>('finance_apply', {
    payload: {
      context: financeContext(library),
      kind: request.kind,
      id: request.id,
      action: request.action ?? 'save',
      values: request.values ?? {},
      requestId: request.requestId,
    },
  })
}

export function getManage(library: NotiaLibrary): Promise<ManageSection[]> {
  return callBackend<ManageSection[]>('finance_manage', { context: financeContext(library) })
}

export function resolveReview(library: NotiaLibrary, reviewItemId: string, optionId: string): Promise<ApplyResult> {
  return callBackend<ApplyResult>('finance_resolve_review', { payload: { context: financeContext(library), reviewItemId, optionId } })
}

export function searchHistory(library: NotiaLibrary, request: { search: string; filter: string; page: number }): Promise<FinanceSearch> {
  return callBackend<FinanceSearch>('finance_search', { payload: { context: financeContext(library), ...request } })
}

export function exportFinance(library: NotiaLibrary, request: { from: string | null; to: string | null; format: 'xlsx' | 'csv' }): Promise<ExportFile> {
  return callBackend<ExportFile>('finance_export', { payload: { context: financeContext(library), ...request } })
}

/** An id for one load, so a retry after a lost answer is not applied twice. */
export function newRequestId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  return `r-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}
