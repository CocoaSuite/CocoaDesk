import type { Currency, Money, Salary } from '../api'

const usdWhole = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 })
const usdCents = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 })
const rub = new Intl.NumberFormat('ru-RU', { style: 'currency', currency: 'RUB', maximumFractionDigits: 0 })

/** "$288", or "$90.50" when there are cents. */
export function fmtUsd(v: number): string {
  return Math.abs(v - Math.round(v)) < 0.005 ? usdWhole.format(Math.round(v)) : usdCents.format(v)
}
/** "23 040 ₽" — rubles are always shown whole. */
export function fmtRub(v: number): string {
  return rub.format(Math.round(v))
}
export const fmtIn = (c: Currency, v: number) => (c === 'usd' ? fmtUsd(v) : fmtRub(v))

/** The same sums as the Rust side, for live figures while editing a card. */
export function money(s: Salary, boardCommission: number, rate: number | null): Money {
  const commission = Math.min(100, Math.max(0, s.commission ?? boardCommission))
  const keep = 1 - commission / 100
  const r = rate && rate > 0 ? rate : null
  const grossUsd = s.currency === 'usd' ? s.amount : r ? s.amount / r : null
  const grossRub = s.currency === 'rub' ? s.amount : r ? s.amount * r : null
  return { commission, grossUsd, grossRub, netUsd: grossUsd == null ? null : grossUsd * keep, netRub: grossRub == null ? null : grossRub * keep, rate: r }
}

/** Adds up net amounts. A side stays known only while every card has it. */
export interface Sum { usd: number; rub: number; usdKnown: boolean; rubKnown: boolean; n: number }
export const emptySum = (): Sum => ({ usd: 0, rub: 0, usdKnown: true, rubKnown: true, n: 0 })
export function addMoney(s: Sum, m: Money | null, sign = 1): Sum {
  if (!m) return s
  s.n += sign
  if (m.netUsd == null) s.usdKnown = false; else s.usd += sign * m.netUsd
  if (m.netRub == null) s.rubKnown = false; else s.rub += sign * m.netRub
  return s
}
/** The main figure of a sum: dollars when they can be given, otherwise rubles. */
export function fmtSumMain(s: Sum): string {
  return s.usdKnown ? fmtUsd(s.usd) : fmtRub(s.rub)
}
export function fmtSumOther(s: Sum): string {
  return s.usdKnown && s.rubKnown ? fmtRub(s.rub) : ''
}
