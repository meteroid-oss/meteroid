import { Skeleton } from '@md/ui'
import { ResponsiveBar, type BarTooltipProps } from '@nivo/bar'
import { useCallback, useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getSubscriptionComponentUsage } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

export interface UsageChartData {
  dataPoints: {
    windowStart: string
    windowEnd: string
    value: string
    dimensions: Record<string, string>
  }[]
  periodStart: string
  periodEnd: string
}

interface UsageBarChartProps {
  subscriptionId: string
  metricId: string
  /** When set, only data points matching these dimensions are shown (usage grouping filter). */
  groupByDimensions?: Record<string, string>
}

/** Fetches via admin RPC and renders the chart */
export const UsageBarChart = ({
  subscriptionId,
  metricId,
  groupByDimensions,
}: UsageBarChartProps) => {
  const usageQuery = useQuery(
    getSubscriptionComponentUsage,
    { subscriptionId, metricId },
    { enabled: Boolean(subscriptionId) && Boolean(metricId) }
  )

  if (usageQuery.isLoading) {
    return <Skeleton height={160} />
  }

  if (usageQuery.isError) {
    return (
      <div className="text-xs text-muted-foreground py-4 text-center">
        Failed to load usage data
      </div>
    )
  }

  if (!usageQuery.data?.dataPoints?.length) {
    return (
      <div className="text-xs text-muted-foreground py-4 text-center">No usage data available</div>
    )
  }

  return <UsageBarChartDisplay data={usageQuery.data} groupByDimensions={groupByDimensions} />
}

/** Pure display component — accepts pre-fetched data */
export const UsageBarChartDisplay = ({
  data,
  groupByDimensions,
}: {
  data: UsageChartData
  groupByDimensions?: Record<string, string>
}) => {
  const { chartData, keys, colorMap } = useMemo(() => {
    if (!data.dataPoints?.length) {
      return { chartData: [], keys: [] as string[], colorMap: new Map<string, string>() }
    }

    const groupByKeys = new Set(Object.keys(groupByDimensions ?? {}))

    const dimensionKeys = new Set<string>()
    const grouped = new Map<string, Record<string, number>>()

    for (const point of data.dataPoints) {
      if (groupByDimensions) {
        const matches = Object.entries(groupByDimensions).every(
          ([k, v]) => point.dimensions[k] === v
        )
        if (!matches) continue
      }

      const dateKey = point.windowStart
      if (!grouped.has(dateKey)) {
        grouped.set(dateKey, {})
      }
      const entry = grouped.get(dateKey)!

      // Remaining dimensions sorted by key for consistent dim1/dim2 ordering
      const remainingDims = Object.entries(point.dimensions)
        .filter(([k]) => !groupByKeys.has(k))
        .sort(([a], [b]) => a.localeCompare(b))

      if (remainingDims.length === 0) {
        const key = 'usage'
        dimensionKeys.add(key)
        entry[key] = (entry[key] || 0) + Number(point.value)
      } else {
        const dimLabel = remainingDims.map(([, v]) => v).join(' / ')
        dimensionKeys.add(dimLabel)
        entry[dimLabel] = (entry[dimLabel] || 0) + Number(point.value)
      }
    }

    const allDays: string[] = []
    if (data.periodStart && data.periodEnd) {
      const start = new Date(data.periodStart + 'T00:00:00')
      const end = new Date(data.periodEnd + 'T00:00:00')
      for (const d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
        allDays.push(d.toISOString().slice(0, 10))
      }
    } else {
      allDays.push(...grouped.keys())
    }

    // Sort keys so stacking order matches the subline list order (dim1 / dim2 alphabetical)
    const keys = Array.from(dimensionKeys).sort()
    const chartData = allDays.map(day => {
      const values = grouped.get(day) ?? {}
      const filled: Record<string, number | string> = {}
      for (const k of keys) {
        filled[k] = values[k] ?? 0
      }
      return { date: formatShortDate(day), rawDate: day, ...filled }
    })

    const colorMap = new Map<string, string>()
    keys.forEach((k, i) => {
      colorMap.set(k, COLORS[i % COLORS.length])
    })

    return { chartData, keys, colorMap }
  }, [data, groupByDimensions])

  const barTooltip = useCallback(
    ({ data: barData }: BarTooltipProps<Record<string, number | string>>) => {
      const rawDate = barData.rawDate as string
      const fullDate = formatFullDate(rawDate)
      const entries = keys
        .map(k => ({ label: k, value: (barData[k] as number) ?? 0, color: colorMap.get(k)! }))
        .filter(e => e.value > 0)
      const total = entries.reduce((sum, e) => sum + e.value, 0)

      return (
        <div className="bg-popover text-popover-foreground border border-border rounded px-2.5 py-1.5 text-xs shadow-sm min-w-[140px]">
          <div className="font-medium mb-1">{fullDate}</div>
          {entries.map(e => (
            <div key={e.label} className="flex items-center justify-between gap-3 py-px">
              <span className="flex items-center gap-1.5">
                <span
                  className="inline-block w-2 h-2 rounded-full shrink-0"
                  style={{ backgroundColor: e.color }}
                />
                {e.label}
              </span>
              <span className="tabular-nums font-medium">{formatCompact(e.value)}</span>
            </div>
          ))}
          {entries.length > 1 && (
            <div className="flex items-center justify-between gap-3 pt-1 mt-1 border-t border-border/50 font-medium">
              <span>Total</span>
              <span className="tabular-nums">{formatCompact(total)}</span>
            </div>
          )}
        </div>
      )
    },
    [keys, colorMap]
  )

  if (chartData.length === 0) {
    return (
      <div className="text-xs text-muted-foreground py-4 text-center">No usage data available</div>
    )
  }

  const showLegend = keys.length > 1

  // Show every Nth x-axis label to avoid crowding
  const tickInterval = chartData.length <= 14 ? 1 : chartData.length <= 31 ? 7 : 14
  const visibleTicks = new Set(
    chartData
      .map((d, i) => (i % tickInterval === 0 || i === chartData.length - 1 ? d.date : null))
      .filter(Boolean)
  )

  return (
    <div>
      <div style={{ height: 170 }}>
        <ResponsiveBar
          data={chartData}
          keys={keys}
          indexBy="date"
          margin={{ top: 10, right: 10, bottom: 30, left: 50 }}
          padding={0.3}
          colors={bar => colorMap.get(bar.id as string) ?? '#888'}
          axisBottom={{
            tickSize: 0,
            tickPadding: 8,
            tickRotation: 0,
            format: v => (visibleTicks.has(v as string) ? (v as string) : ''),
          }}
          axisLeft={{
            tickSize: 0,
            tickPadding: 8,
            format: v => formatCompact(Number(v)),
          }}
          enableLabel={false}
          enableGridY={true}
          tooltip={barTooltip}
          theme={{
            text: { fill: 'hsl(var(--muted-foreground))', fontSize: 10 },
            grid: { line: { stroke: 'hsl(var(--border))', strokeWidth: 1 } },
            axis: { ticks: { text: { fill: 'hsl(var(--muted-foreground))' } } },
          }}
        />
      </div>
      {showLegend && (
        <div className="flex flex-wrap gap-x-3 gap-y-1 px-1 pt-1 pb-0.5">
          {keys.map(k => (
            <span
              key={k}
              className="inline-flex items-center gap-1 text-[10px] text-muted-foreground"
            >
              <span
                className="inline-block w-2 h-2 rounded-full shrink-0"
                style={{ backgroundColor: colorMap.get(k) }}
              />
              {k}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

const COLORS = [
  '#4e79a7', '#f28e2b', '#e15759', '#76b7b2', '#59a14f',
  '#edc948', '#b07aa1', '#ff9da7', '#9c755f', '#bab0ac',
  '#86bcb6', '#8cd17d', '#b6992d', '#499894', '#d37295',
  '#a0cbe8', '#ffbe7d', '#d4a6c8', '#fabfd2', '#d7b5a6',
]

function formatShortDate(dateStr: string): string {
  try {
    const d = new Date(dateStr + 'T00:00:00')
    return `${d.toLocaleString('en-US', { month: 'short' })} ${d.getDate()}`
  } catch {
    return dateStr
  }
}

function formatFullDate(dateStr: string): string {
  try {
    const d = new Date(dateStr + 'T00:00:00')
    return d.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' })
  } catch {
    return dateStr
  }
}

function formatCompact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toFixed(n % 1 === 0 ? 0 : 2)
}
