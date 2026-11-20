// What the tests of the screen get from Rust.
import type { ManageSection } from '../types/financeEntry'
import type { FinanceMovements, FinanceOverview, FinancePayments, FinanceSavings } from '../types/financeScreen'

const nav = { month: '2026-11', label: 'noviembre 2026', previous: '2026-10', next: '2026-12', isCurrent: true }
const pesos = (amount: string) => ({ currency: 'ARS', amount })

export const overview: FinanceOverview = {
  ...nav,
  balance: [{ currency: 'ARS', income: '1500000.00', salary: '1500000.00', otherIncome: '0.00', expenses: '400000.00', saved: '100000.00', left: '1000000.00', spentPercent: 26.7, savedPercent: 6.7, previousExpenses: '500000.00', expensesChange: -20 }],
  pace: { daysElapsed: 20, daysInMonth: 30, dailyAverage: '20000.00', projected: '600000.00', expenseCount: 12, biggest: null },
  commitments: {
    items: [
      { kind: 'service', id: 's1', title: 'Internet', subtitle: 'Servicio', amount: '25000.00', currency: 'ARS', status: 'pending' },
      { kind: 'card', id: 'c1', title: 'Visa', subtitle: '2 cuotas', amount: '90000.00', currency: 'ARS', status: 'late' },
      { kind: 'loan', id: 'l1', title: 'Préstamo', subtitle: 'Cuota 2 de 12', amount: '62000.00', currency: 'ARS', status: 'paid' },
    ],
    paid: [pesos('62000.00')],
    pending: [pesos('115000.00')],
    paidCount: 1,
    pendingCount: 2,
  },
  categories: [{ categoryId: 'category-supermarket', name: 'Supermercado', currency: 'ARS', amount: '280000.00', percent: 70, count: 4, budget: { id: 'b1', categoryId: 'category-supermarket', categoryName: 'Supermercado', currency: 'ARS', budget: '250000.00', spent: '280000.00', remaining: '-30000.00', percent: 112, status: 'over' } }],
  history: ['06', '07', '08', '09', '10', '11'].map((month) => ({ month: `2026-${month}`, label: `${month}`, income: '1500000.00', expenses: '400000.00', saved: '0.00' })),
  debts: { cards: [{ id: 'c1', name: 'Visa', remaining: [pesos('540000.00')], installmentsLeft: 9, lastMonth: 'agosto 2027' }], cardTotal: [pesos('540000.00')], upcoming: [], loans: [] },
  savings: [{ currency: 'ARS', balance: '300000.00', savedThisMonth: '100000.00' }],
  topExpenses: [{ id: 'm1', kind: 'expense', name: 'Coto', categoryId: 'category-supermarket', category: 'Supermercado', account: null, amount: '90000.00', currency: 'ARS', date: '2026-11-05', detail: null, tag: null }],
  recent: [],
  empty: false,
}

export const payments: FinancePayments = {
  ...nav,
  services: [{ id: 's1', name: 'Internet', currency: 'ARS', amount: '25000.00', paid: null }],
  cards: [{
    id: 'c1',
    name: 'Visa',
    installments: [
      { purchaseId: 'p1', name: 'Heladera', number: 3, count: 12, month: '2026-11', amount: '60000.00', currency: 'ARS', paid: null, late: false },
      { purchaseId: 'p2', name: 'Zapatillas', number: 1, count: 3, month: '2026-10', amount: '30000.00', currency: 'ARS', paid: null, late: true },
    ],
    total: [pesos('90000.00')],
    pending: [pesos('90000.00')],
    pendingCount: 2,
    debt: [pesos('600000.00')],
    purchases: [{ id: 'p1', name: 'Heladera', category: 'Hogar', total: '720000.00', currency: 'ARS', count: 12, paidCount: 2, remaining: '600000.00', firstMonth: '2026-09', lastMonth: '2027-08', finished: false }],
  }],
  loans: [{ id: 'l1', name: 'Préstamo', currency: 'ARS', amount: '600000.00', count: 12, number: 2, paid: null, nextNumber: 2, lastPaid: '62000.00', paidCount: 1, totalPaid: '62000.00', lateCount: 0, finished: false }],
  paid: [],
  pending: [pesos('177000.00')],
}

export const movements: FinanceMovements = {
  ...nav,
  rows: overview.topExpenses,
  income: [],
  expenses: [pesos('90000.00')],
  categories: [{ id: 'category-supermarket', name: 'Supermercado' }],
  accounts: [],
}

export const savings: FinanceSavings = {
  reserves: [{ id: 'reserve-pesos', name: 'Ahorro en pesos', currency: 'ARS', balance: '300000.00', goal: '1000000.00', goalPercent: 30, savedThisMonth: '100000.00' }],
  totals: [{ currency: 'ARS', balance: '300000.00', savedThisMonth: '100000.00' }],
  dollarsInPesos: null,
  movements: [],
  quotes: [],
  quotesError: 'sin red',
}

export const manage: ManageSection[] = [
  { key: 'cards', title: 'Tarjetas', addKind: 'card', addLabel: 'Nueva tarjeta', empty: 'Sin tarjetas.', rows: [{ id: 'c1', kind: 'card', title: 'Visa', subtitle: '1 compra en curso', muted: false }] },
  { key: 'expense-categories', title: 'Categorías de gastos', addKind: 'category', addLabel: 'Nueva categoría', empty: 'Sin categorías.', rows: [{ id: 'category-supermarket', kind: 'category', title: 'Supermercado', subtitle: '4 movimientos', muted: false }] },
]
