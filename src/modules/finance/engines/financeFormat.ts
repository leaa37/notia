// How the Finanzas screen writes the figures Rust derives: money, percents
// and dates in Argentine Spanish. Presentation only; nothing here decides a
// figure.

import type { Money } from '../types/financeScreen'

const MONTHS = ['enero', 'febrero', 'marzo', 'abril', 'mayo', 'junio', 'julio', 'agosto', 'septiembre', 'octubre', 'noviembre', 'diciembre']
const SHORT_MONTHS = ['ene', 'feb', 'mar', 'abr', 'may', 'jun', 'jul', 'ago', 'sep', 'oct', 'nov', 'dic']

function number(value: number, decimals: number): string {
  return value.toLocaleString('es-AR', { minimumFractionDigits: decimals, maximumFractionDigits: decimals })
}

/** `$ 5.250.432,63` or `US$ 1.000,00`; `decimals: 0` rounds to whole units. */
export function formatMoney(money: Money, decimals: 0 | 2 = 2): string {
  const value = Number(money.amount)
  const prefix = money.currency === 'ARS' ? '$' : money.currency === 'USD' ? 'US$' : money.currency
  if (!Number.isFinite(value)) return `${prefix} ${money.amount}`
  const sign = value < 0 ? '−' : ''
  return `${sign}${prefix} ${number(Math.abs(value), decimals)}`
}

export function formatAmount(amount: string | number, currency: string, decimals: 0 | 2 = 2): string {
  return formatMoney({ amount: String(amount), currency }, decimals)
}

/** Cents only when there are: `$ 60.000` or `$ 33.333,34`. */
export function formatExact(amount: string | number, currency: string): string {
  return formatAmount(amount, currency, Number.isInteger(Number(amount)) ? 0 : 2)
}

/** Each currency apart; `—` when there is nothing. */
export function formatMoneyList(list: Money[], decimals: 0 | 2 = 2): string {
  return list.length === 0 ? '—' : list.map((money) => formatMoney(money, decimals)).join(' · ')
}

export function formatPercent(value: number | null | undefined, decimals = 1): string {
  return value === null || value === undefined || !Number.isFinite(value) ? '—' : `${number(value, decimals)} %`
}

export function formatSignedPercent(value: number, decimals = 1): string {
  return `${value > 0 ? '+' : value < 0 ? '−' : ''}${formatPercent(Math.abs(value), decimals)}`
}

/** `6,11 M`. */
export function formatMillions(value: number): string {
  return `${number(value / 1_000_000, 2)} M`
}

function monthIndex(period: string): number {
  return Number(period.slice(5, 7)) - 1
}

/** `Septiembre 2026`. */
export function formatMonthTitle(period: string): string {
  const name = MONTHS[monthIndex(period)] ?? period
  return `${name.charAt(0).toUpperCase()}${name.slice(1)} ${period.slice(0, 4)}`
}

/** `agosto`. */
export function formatMonthName(period: string): string {
  return MONTHS[monthIndex(period)] ?? period
}

/** `ago`. */
export function formatShortMonth(period: string): string {
  return SHORT_MONTHS[monthIndex(period)] ?? period
}

/** `oct 2025`. */
export function formatShortMonthYear(period: string): string {
  return `${formatShortMonth(period)} ${period.slice(0, 4)}`
}

/** `5 ago`. */
export function formatShortDate(date: string): string {
  const day = Number(date.slice(8, 10))
  return Number.isFinite(day) && day > 0 ? `${day} ${formatShortMonth(date)}` : date
}

/** `5 de agosto de 2026`. */
export function formatLongDate(date: string): string {
  const day = Number(date.slice(8, 10))
  return Number.isFinite(day) && day > 0 ? `${day} de ${formatMonthName(date)} de ${date.slice(0, 4)}` : date
}

/** `24 sep, 15:00` from an ISO timestamp; the date alone when it has no time. */
export function formatUpdatedAt(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  const time = date.toLocaleTimeString('es-AR', { hour: '2-digit', minute: '2-digit', hour12: false })
  return `${date.getDate()} ${SHORT_MONTHS[date.getMonth()]}, ${time}`
}

export function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : pluralForm}`
}
