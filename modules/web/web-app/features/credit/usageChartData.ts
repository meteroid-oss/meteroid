import { UsageCostPeriod } from '@/rpc/api/wallets/v1/wallets_pb'

import type { WalletDailyUsageCost } from '@/rpc/api/wallets/v1/models_pb'

const DAY_MS = 24 * 60 * 60 * 1000
// A billing period is at most a year; the guard only stops a malformed range.
const MAX_DAYS = 400

export type UsageRange = 'current' | 'last'

/**
 * The request for the wallet's daily usage over a period: every subscription's, or `scope`'s
 * only. The period bar and the chart send the same one, so they read one response.
 */
export const walletDailyUsageInput = (
  wallet: { id: string; customerId: string },
  scope: string | undefined,
  range: UsageRange
) => ({
  customerId: wallet.customerId,
  walletId: wallet.id,
  subscriptionId: scope,
  period: range === 'current' ? UsageCostPeriod.CURRENT : UsageCostPeriod.PREVIOUS,
})

export type DailyUsageRow = Pick<WalletDailyUsageCost, 'date' | 'metricName' | 'amount'>

/** What the rows cost the wallet in all: the period's usage, as its invoices bill it. */
export const dailyUsageTotal = (rows: Pick<WalletDailyUsageCost, 'amount'>[]): bigint =>
  rows.reduce((sum, row) => sum + row.amount, 0n)

/** Every UTC day from `start` up to `end` (exclusive), as YYYY-MM-DD. */
export const daysBetween = (start: string, end: string): string[] => {
  const days: string[] = []
  const last = Date.parse(`${end}T00:00:00Z`)
  for (
    let at = Date.parse(`${start}T00:00:00Z`);
    at < last && days.length < MAX_DAYS;
    at += DAY_MS
  ) {
    days.push(new Date(at).toISOString().slice(0, 10))
  }
  return days
}

export const OTHER_SERIES = 'Other'

export type UsageSeries = { key: string; total: bigint; share: number }

export type WalletUsageChartData = {
  days: { date: string; values: Record<string, bigint> }[]
  /** Biggest first, the folded remainder last. */
  series: UsageSeries[]
  total: bigint
}

/**
 * The wallet's usage per day and per metric, as the server prices each day: what it adds to the
 * period's price. Past `maxSeries` metrics, the smallest fold into "Other".
 */
export const walletUsageChartData = (
  rows: DailyUsageRow[],
  range: { start: string; end: string } | undefined,
  maxSeries = 5
): WalletUsageChartData => {
  const totals = new Map<string, bigint>()
  for (const row of rows) {
    totals.set(row.metricName, (totals.get(row.metricName) ?? 0n) + row.amount)
  }
  const ranked = [...totals].sort(([a, x], [b, y]) =>
    x === y ? a.localeCompare(b) : x > y ? -1 : 1
  )
  const kept = new Set(
    (ranked.length > maxSeries ? ranked.slice(0, maxSeries - 1) : ranked).map(([key]) => key)
  )
  const keyOf = (name: string) => (kept.has(name) ? name : OTHER_SERIES)

  const byDay = new Map<string, Record<string, bigint>>()
  for (const row of rows) {
    const values = byDay.get(row.date) ?? {}
    const key = keyOf(row.metricName)
    values[key] = (values[key] ?? 0n) + row.amount
    byDay.set(row.date, values)
  }

  const dates = range ? daysBetween(range.start, range.end) : []
  const known = new Set(dates)
  const outside = [...byDay.keys()].filter(day => !known.has(day))
  const days = [...dates, ...outside].sort().map(date => ({ date, values: byDay.get(date) ?? {} }))

  const total = dailyUsageTotal(rows)
  const share = (amount: bigint) => (total > 0n ? Number((amount * 10000n) / total) / 100 : 0)
  const series: UsageSeries[] = ranked
    .filter(([key]) => kept.has(key))
    .map(([key, amount]) => ({ key, total: amount, share: share(amount) }))
  const other = ranked
    .filter(([key]) => !kept.has(key))
    .reduce((sum, [, amount]) => sum + amount, 0n)
  if (ranked.some(([key]) => !kept.has(key))) {
    series.push({ key: OTHER_SERIES, total: other, share: share(other) })
  }
  return { days, series, total }
}

/** Series keys in a stable order, so a series keeps its color whatever its rank. */
export const seriesColorOrder = (series: Pick<UsageSeries, 'key'>[]): string[] =>
  series
    .map(s => s.key)
    .filter(key => key !== OTHER_SERIES)
    .sort((a, b) => a.localeCompare(b))
