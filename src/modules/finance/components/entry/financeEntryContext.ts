import { createContext, useContext } from 'react'
import type { EntryRequest } from '../../types/financeEntry'

export interface FinanceEntryApi {
  /** Opens the form of a new load or of a stored record. */
  openEntry: (request: EntryRequest) => void
  /** Opens the «Cargar» menu with every kind of load. */
  openMenu: () => void
  /** Answers a «Para revisar» case with one of its options. */
  resolve: (reviewItemId: string, optionId: string) => Promise<void>
  /** Opens the export of the movements, starting at `month`. */
  openExport: (month: string) => void
}

export const FinanceEntryContext = createContext<FinanceEntryApi | null>(null)

/** The manual loads of the screen; `null` outside Finanzas (a lone tab in a test). */
export function useFinanceEntry(): FinanceEntryApi | null {
  return useContext(FinanceEntryContext)
}

