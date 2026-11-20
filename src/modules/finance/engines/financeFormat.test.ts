import { describe, expect, it } from 'vitest'
import {
  formatLongDate,
  formatMillions,
  formatMoney,
  formatMoneyList,
  formatMonthTitle,
  formatPercent,
  formatShortDate,
  formatShortMonthYear,
  formatSignedPercent,
} from './financeFormat'

describe('financeFormat', () => {
  it('writes money per currency, as the canvas does', () => {
    expect(formatMoney({ currency: 'ARS', amount: '5250432.63' })).toBe('$ 5.250.432,63')
    expect(formatMoney({ currency: 'USD', amount: '1000.00' }, 0)).toBe('US$ 1.000')
    expect(formatMoneyList([])).toBe('—')
    expect(formatMoneyList([{ currency: 'ARS', amount: '10' }, { currency: 'USD', amount: '2' }], 0)).toBe('$ 10 · US$ 2')
  })

  it('writes percents and dates in Spanish', () => {
    expect(formatPercent(12.7)).toBe('12,7 %')
    expect(formatPercent(null)).toBe('—')
    expect(formatSignedPercent(1.3)).toBe('+1,3 %')
    expect(formatMillions(6_107_053.29)).toBe('6,11 M')
    expect(formatMonthTitle('2026-09')).toBe('Septiembre 2026')
    expect(formatShortDate('2026-08-05')).toBe('5 ago')
    expect(formatLongDate('2026-08-05')).toBe('5 de agosto de 2026')
    expect(formatShortMonthYear('2025-10')).toBe('oct 2025')
  })
})
