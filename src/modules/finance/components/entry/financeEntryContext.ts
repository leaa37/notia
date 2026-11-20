import { createContext, useContext } from 'react'
import type { EntryRequest, EntryValues } from '../../types/financeEntry'

export interface FinanceEntryApi {
  /** Opens the form of a new load or of a stored record. */
  openEntry: (request: EntryRequest) => void
  /** Opens the «Cargar» menu with every kind of load. */
  openMenu: () => void
  /** Opens the export of the movements, starting at `month`. */
  openExport: (month: string) => void
  /** Applies a load without a form (paying an installment with one tap), with «Deshacer». */
  quickApply: (kind: string, values: EntryValues) => Promise<boolean>
}

export const FinanceEntryContext = createContext<FinanceEntryApi | null>(null)

/** The manual loads of the screen; `null` outside Finanzas (a lone tab in a test). */
export function useFinanceEntry(): FinanceEntryApi | null {
  return useContext(FinanceEntryContext)
}
