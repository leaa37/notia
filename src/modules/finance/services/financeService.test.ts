import { callBackend } from '../../../services/transport'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { clearAllFinanceData, getFinanceMovements, getFinanceOverview, getFinancePayments, getFinanceSavings, searchFinanceHistory } from './financeService'
import type { NotiaLibrary } from '../../../types/notia'

vi.mock('../../../services/transport', () => ({ callBackend: vi.fn() }))

describe('financeService', () => {
  beforeEach(() => vi.clearAllMocks())

  it('sends the library context and the month through the typed commands', async () => {
    vi.mocked(callBackend).mockResolvedValue({})
    const library: NotiaLibrary = { id: 'library-1', name: 'Personal', path: 'C:/personal', androidTreeUri: 'content://personal' }
    const context = { libraryPath: library.path, androidDirectoryUri: library.androidTreeUri, actorLibraryUserId: 'user-owner', source: 'app' }
    const filter = { kind: 'expense' as const, categoryId: '', accountId: '', search: 'nafta' }

    await getFinanceOverview(library, '2026-11')
    await getFinancePayments(library, '2026-11')
    await getFinanceMovements(library, '2026-11', filter)
    await searchFinanceHistory(library, filter, 2)
    await getFinanceSavings(library)

    expect(callBackend).toHaveBeenNthCalledWith(1, 'finance_overview', { payload: { context, month: '2026-11' } })
    expect(callBackend).toHaveBeenNthCalledWith(2, 'finance_payments', { payload: { context, month: '2026-11' } })
    expect(callBackend).toHaveBeenNthCalledWith(3, 'finance_movements', { payload: { context, month: '2026-11', filter } })
    expect(callBackend).toHaveBeenNthCalledWith(4, 'finance_search', { payload: { context, filter, page: 2 } })
    expect(callBackend).toHaveBeenNthCalledWith(5, 'finance_savings', { payload: { context } })
  })

  it('clears finance data only through the native command for the active library', async () => {
    vi.mocked(callBackend).mockResolvedValue(undefined)
    const library: NotiaLibrary = { id: 'library-1', name: 'Personal', path: 'C:/personal', androidTreeUri: 'content://personal' }
    await clearAllFinanceData(library)
    expect(callBackend).toHaveBeenCalledWith('finance_clear_all_data', {
      context: { libraryPath: library.path, androidDirectoryUri: library.androidTreeUri, actorLibraryUserId: 'user-owner', source: 'app' },
    })
  })
})
