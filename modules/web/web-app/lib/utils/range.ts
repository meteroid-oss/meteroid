import { startOfMonth, startOfQuarter, startOfYear, subDays, subHours, subMonths } from 'date-fns'

/**
 * Relative time-range presets, stored (never absolute) and resolved against "now"
 * at view/read time — so a saved tile or a shared link always tracks the present.
 * The keys are the contract shared with the Rust resolver (portal dashboard service).
 */
export type RangeKey = '24h' | '7d' | '30d' | '3m' | '6m' | '12m' | 'mtd' | 'qtd' | 'ytd'

export const DEFAULT_RANGE: RangeKey = '12m'

export const RANGE_PRESETS: { key: RangeKey; label: string }[] = [
  { key: '24h', label: 'Last 24 hours' },
  { key: '7d', label: 'Last 7 days' },
  { key: '30d', label: 'Last 30 days' },
  { key: '3m', label: 'Last 3 months' },
  { key: '6m', label: 'Last 6 months' },
  { key: '12m', label: 'Last 12 months' },
  { key: 'mtd', label: 'Month to date' },
  { key: 'qtd', label: 'Quarter to date' },
  { key: 'ytd', label: 'Year to date' },
]

const KEYS = new Set<string>(RANGE_PRESETS.map(r => r.key))

export const asRangeKey = (v: string | undefined | null): RangeKey =>
  v && KEYS.has(v) ? (v as RangeKey) : DEFAULT_RANGE

export const rangeLabel = (k: RangeKey): string =>
  RANGE_PRESETS.find(r => r.key === k)?.label ?? k

export const resolveRange = (key: RangeKey, now: Date = new Date()): { from: Date; to: Date } => {
  const from = (() => {
    switch (key) {
      case '24h':
        return subHours(now, 24)
      case '7d':
        return subDays(now, 7)
      case '30d':
        return subDays(now, 30)
      case '3m':
        return subMonths(now, 3)
      case '6m':
        return subMonths(now, 6)
      case 'mtd':
        return startOfMonth(now)
      case 'qtd':
        return startOfQuarter(now)
      case 'ytd':
        return startOfYear(now)
      case '12m':
      default:
        return subMonths(now, 12)
    }
  })()
  return { from, to: now }
}

/** Preset list for the date-range picker's shortcut column, resolved lazily against "now". */
export const REPORT_RANGE_PRESETS = RANGE_PRESETS.map(p => ({
  label: p.label,
  getRange: () => resolveRange(p.key),
}))
