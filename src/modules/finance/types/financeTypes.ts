import type { NotiaLibrary } from '../../../types/notia'

export type FinanceCurrency = 'ARS' | 'USD'

export interface FinanceContext {
  libraryPath: string
  androidDirectoryUri?: string
  actorLibraryUserId: string
  source: 'app' | 'public-url' | 'telegram'
}

/** The Owner's context of `library`, as every Finanzas command takes it. */
export function financeContext(
  library: NotiaLibrary,
  actorLibraryUserId = 'user-owner',
  source: FinanceContext['source'] = 'app',
): FinanceContext {
  return {
    libraryPath: library.path,
    androidDirectoryUri: library.androidTreeUri,
    actorLibraryUserId,
    source,
  }
}

export interface FinanceDevTable {
  name: string
}

export interface FinanceDevQueryResult {
  columns: string[]
  rows: Array<Array<string | null>>
  totalRows: number
  page: number
  pageSize: number
}
