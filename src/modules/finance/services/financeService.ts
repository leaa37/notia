import { callBackend } from '../../../services/transport'
import type { NotiaLibrary } from '../../../types/notia'
import type { FinanceDevQueryResult, FinanceDevTable } from '../types/financeTypes'
import { financeContext } from '../types/financeTypes'
import type { FinanceMovements, FinanceOverview, FinancePayments, FinanceSavings, FinanceSearch, MovementFilter } from '../types/financeScreen'

// The tabs of Finanzas read what `finance_screen.rs` builds; nothing is
// computed here.

/** «Resumen» of a month. */
export function getFinanceOverview(library: NotiaLibrary, month: string): Promise<FinanceOverview> {
  return callBackend<FinanceOverview>('finance_overview', { payload: { context: financeContext(library), month } })
}

/** «Pagos»: services, cards and loans of a month. */
export function getFinancePayments(library: NotiaLibrary, month: string): Promise<FinancePayments> {
  return callBackend<FinancePayments>('finance_payments', { payload: { context: financeContext(library), month } })
}

/** «Movimientos» of a month, filtered by Rust. */
export function getFinanceMovements(library: NotiaLibrary, month: string, filter: MovementFilter): Promise<FinanceMovements> {
  return callBackend<FinanceMovements>('finance_movements', { payload: { context: financeContext(library), month, filter } })
}

/** The whole history, with the filters of «Movimientos». */
export function searchFinanceHistory(library: NotiaLibrary, filter: MovementFilter, page: number): Promise<FinanceSearch> {
  return callBackend<FinanceSearch>('finance_search', { payload: { context: financeContext(library), filter, page } })
}

/** «Ahorro»: reserves, the dollar and their movements. */
export function getFinanceSavings(library: NotiaLibrary): Promise<FinanceSavings> {
  return callBackend<FinanceSavings>('finance_savings', { payload: { context: financeContext(library) } })
}

export function listFinanceDevTables(): Promise<FinanceDevTable[]> {
  return callBackend<FinanceDevTable[]>('finance_dev_list_tables')
}

export function queryFinanceDevTable(library: NotiaLibrary, tableName: string, page: number, pageSize = 50): Promise<FinanceDevQueryResult> {
  return callBackend<FinanceDevQueryResult>('finance_dev_query_table', { payload: { context: financeContext(library), tableName, page, pageSize } })
}

export function queryFinanceDevSql(library: NotiaLibrary, sql: string, page: number, pageSize = 50): Promise<FinanceDevQueryResult> {
  return callBackend<FinanceDevQueryResult>('finance_dev_query_sql', { payload: { context: financeContext(library), sql, page, pageSize } })
}

export function seedFinanceDevData(library: NotiaLibrary): Promise<void> {
  return callBackend('finance_dev_seed_demo_data', { context: financeContext(library) })
}

export function clearAllFinanceData(library: NotiaLibrary): Promise<void> {
  return callBackend('finance_clear_all_data', { context: financeContext(library) })
}
