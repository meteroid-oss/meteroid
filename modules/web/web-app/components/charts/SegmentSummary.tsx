// An interactive per-segment legend for a metric split by an enum-native dimension
// (charge type, movement type, payment method). Colours match the chart series and
// each compact chip carries the segment's headline value: click toggles a segment on
// or off (multi-select), ⌥/⇧-click isolates it. This is what lets `revenue_by_charge_type`
// and `mrr_movements` subsume the standalone component charts they replaced.

import { cn } from '@md/ui'
import { useMemo } from 'react'

import {
  compareSegmentKeys,
  groupValueLabel,
  seriesPalette,
  type ExplorePoint,
} from '@/components/charts/ExploreChart'
import { makeFormatter, scaleValue } from '@/components/charts/format'

import type { MetricUnit } from '@/components/charts/types'

interface SegmentSummaryProps {
  points: ExplorePoint[]
  /** The active (enum) breakdown dimension the points are split by. */
  groupBy: string
  unit: MetricUnit
  currency: string
  /** Stock metrics headline the latest window; flows sum the range. */
  summary: 'sum' | 'latest'
  isDark: boolean
  /** Segment values currently toggled off (hidden from the chart). */
  hidden: Set<string>
  /** Toggle one segment's visibility. */
  onToggle: (value: string) => void
  /** Show only this segment, hiding all others (same value again shows all). */
  onIsolate: (value: string) => void
  /** Clear all hiding — show every segment. */
  onShowAll: () => void
}

export const SegmentSummary = ({
  points,
  groupBy,
  unit,
  currency,
  summary,
  isDark,
  hidden,
  onToggle,
  onIsolate,
  onShowAll,
}: SegmentSummaryProps) => {
  const fmt = useMemo(() => makeFormatter(unit, currency), [unit, currency])
  const palette = seriesPalette(isDark)

  // Sorted to match the chart's series/colour order, each reduced to its headline
  // (latest window for stocks, sum over range for flows) and share of the gross.
  const segments = useMemo(() => {
    const perSeries = new Map<string, Map<string, number>>()
    for (const p of points) {
      const key = p.dimensions[groupBy] ?? 'Unattributed'
      const byWindow = perSeries.get(key) ?? new Map<string, number>()
      byWindow.set(
        p.windowStart,
        (byWindow.get(p.windowStart) ?? 0) + scaleValue(unit, currency, p.value)
      )
      perSeries.set(key, byWindow)
    }
    const headline = (byWindow: Map<string, number>): number => {
      if (summary === 'latest') {
        const last = [...byWindow.keys()].sort().at(-1)
        return last ? (byWindow.get(last) ?? 0) : 0
      }
      return [...byWindow.values()].reduce((sum, v) => sum + v, 0)
    }
    const items = [...perSeries.keys()].sort(compareSegmentKeys).map((key, i) => ({
      key,
      value: headline(perSeries.get(key)!),
      color: palette[i % palette.length],
    }))
    const gross = items.reduce((sum, it) => sum + Math.abs(it.value), 0)
    return items.map(it => ({ ...it, share: gross > 0 ? Math.abs(it.value) / gross : 0 }))
  }, [points, groupBy, unit, currency, summary, palette])

  if (segments.length === 0) return null

  const visibleCount = segments.reduce((n, s) => n + (hidden.has(s.key) ? 0 : 1), 0)
  const anyHidden = hidden.size > 0

  return (
    <div className="flex flex-col gap-1.5 border-t border-border pt-3 mt-4">
      <div className="flex h-5 items-center justify-between">
        <span className="text-xs text-muted-foreground">
          {summary === 'latest' ? 'Latest by segment' : 'Total by segment'}
        </span>
        {anyHidden && (
          <div className="flex items-center gap-2">
            <span className="text-[11px] tabular-nums text-muted-foreground">
              {visibleCount}/{segments.length} shown
            </span>
            <button
              type="button"
              onClick={onShowAll}
              className="text-xs text-muted-foreground hover:text-foreground"
            >
              Show all
            </button>
          </div>
        )}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {segments.map(s => {
          const off = hidden.has(s.key)
          const label = groupValueLabel(groupBy, s.key)
          return (
            <button
              key={s.key}
              type="button"
              // Click toggles a segment on/off; ⌥/⇧-click isolates it (hides all others).
              onClick={e => (e.altKey || e.shiftKey ? onIsolate(s.key) : onToggle(s.key))}
              aria-pressed={!off}
              title={`${label} · ${fmt(s.value)} · ${(s.share * 100).toFixed(0)}% of total`}
              className={cn(
                'inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs transition-colors',
                off
                  ? 'border-dashed border-border text-muted-foreground'
                  : 'border-border hover:bg-muted/40'
              )}
            >
              <span
                className="h-2 w-2 shrink-0 rounded-full"
                style={{
                  backgroundColor: off ? 'transparent' : s.color,
                  boxShadow: off ? `inset 0 0 0 1.5px ${s.color}` : undefined,
                }}
              />
              <span className={cn('max-w-[140px] truncate', off && 'line-through')}>{label}</span>
              <span
                className={cn(
                  'tabular-nums',
                  off ? 'opacity-60' : 'font-medium text-foreground'
                )}
              >
                {fmt(s.value)}
              </span>
            </button>
          )
        })}
      </div>
      {segments.length > 1 && (
        <span className="text-[11px] text-muted-foreground pt-2.5 -mb-2.5">
          Click to toggle · ⌥/⇧-click to isolate
        </span>
      )}
    </div>
  )
}
