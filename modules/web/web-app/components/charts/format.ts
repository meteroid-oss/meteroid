// Chart value formatting. Money values arrive as integer MINOR units (cents) in the
// currency the response reports; counts as plain numbers. Never a hardcoded symbol.

import { format as formatDate, parseISO } from 'date-fns'

import { currencyPrecision } from '@/lib/data/currency-precision'

import type { MetricUnit, Polarity } from '@/components/charts/types'

const precisionFor = (currency: string) => currencyPrecision(currency)

/** Scale a raw point value to its display unit (minor units -> major for money). */
export const scaleValue = (unit: MetricUnit, currency: string, raw: number): number => {
  if (unit === 'currency') return raw / Math.pow(10, precisionFor(currency))
  return raw
}

/** A full-precision value formatter for a metric, bound to the response currency. */
export const makeFormatter = (unit: MetricUnit, currency: string) => {
  switch (unit) {
    case 'count':
      return (v: number) => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(v)
    case 'percent':
      return (v: number) => `${v.toFixed(1)}%`
    case 'currency':
    default: {
      const precision = precisionFor(currency)
      return (v: number) =>
        new Intl.NumberFormat(undefined, {
          style: 'currency',
          currency,
          currencyDisplay: 'narrowSymbol',
          maximumFractionDigits: Math.abs(v) >= 1000 ? 0 : precision,
        }).format(v)
    }
  }
}

export type DeltaTone = 'positive' | 'negative' | 'neutral'

/** Tone of a percent move by the metric's polarity — so churn rising reads red, not green. */
export const deltaTone = (deltaPct: number, polarity: Polarity): DeltaTone => {
  if (polarity === 'neutral' || deltaPct === 0) return 'neutral'
  const good = polarity === 'higher_is_better' ? deltaPct > 0 : deltaPct < 0
  return good ? 'positive' : 'negative'
}

export const toneClass: Record<DeltaTone, string> = {
  positive: 'text-success',
  negative: 'text-destructive',
  neutral: 'text-muted-foreground',
}

/** KPI caption: "as of <day>" for a stock, "<first> – <last>" for a flow. */
export const kpiPeriodLabel = (summary: 'sum' | 'latest', windows: string[]): string => {
  if (windows.length === 0) return summary === 'latest' ? 'latest value' : 'total over range'
  const sorted = [...windows].sort()
  const last = parseISO(sorted[sorted.length - 1])
  if (summary === 'latest') return `as of ${formatDate(last, 'MMM d, yyyy')}`
  const first = parseISO(sorted[0])
  return `${formatDate(first, 'MMM d')} – ${formatDate(last, 'MMM d, yyyy')}`
}

/** A compact axis formatter (e.g. €1.2k, 3.4k) that keeps the sign before the symbol. */
export const makeAxisFormatter = (unit: MetricUnit, currency: string) => {
  const compact = (v: number) =>
    new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(v)
  if (unit !== 'currency') return (v: number) => compact(v)
  // Pull the currency's narrow symbol once via a formatted-parts probe.
  const symbol =
    new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      currencyDisplay: 'narrowSymbol',
    })
      .formatToParts(0)
      .find(p => p.type === 'currency')?.value ?? ''
  return (v: number) => `${v < 0 ? '-' : ''}${symbol}${compact(Math.abs(v))}`
}
