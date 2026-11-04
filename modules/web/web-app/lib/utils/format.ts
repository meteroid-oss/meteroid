import { parseISO } from 'date-fns'

import { formatCurrency } from '@/lib/utils/numbers'

export type MoneyFormatter = (amountMinor: number, currency: string) => string

/** Whole amounts drop their decimals (`€800`, `€1,234`); others keep them (`€72.50`). */
export const formatMoney: MoneyFormatter = (amountMinor, currency) =>
  formatCurrency(amountMinor, currency, { trimWholeAmounts: true })

/** `20000` → `20k`, `1250000` → `1.3M`, `12.345` → `12.35` (same rule as the server). */
export const compactQuantity = (quantity: string | number): string => {
  const q = Number(quantity)
  if (!Number.isFinite(q)) return String(quantity)
  const abs = Math.abs(q)
  const [scaled, suffix] =
    abs >= 1e9
      ? [q / 1e9, 'B']
      : abs >= 1e6
        ? [q / 1e6, 'M']
        : abs >= 1e3
          ? [q / 1e3, 'k']
          : [q, '']
  const factor = 10 ** (suffix ? 1 : 2)
  const rounded = (Math.sign(scaled) * Math.round(Math.abs(scaled) * factor)) / factor
  return `${rounded}${suffix}`
}

export const formatQuantity = (quantity: string | number) => {
  const q = Number(quantity)
  return Number.isFinite(q)
    ? q.toLocaleString(undefined, { maximumFractionDigits: 2 })
    : String(quantity)
}

export const plural = (n: number, unit: string) => (n === 1 ? `1 ${unit}` : `${n} ${unit}s`)

/** Server instants are naive UTC. */
export const parseInstant = (instant: string) =>
  parseISO(/[zZ]|[+-]\d\d:?\d\d$/.test(instant) ? instant : `${instant}Z`)
