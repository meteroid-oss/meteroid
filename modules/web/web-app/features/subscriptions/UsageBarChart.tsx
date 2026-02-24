import { Skeleton } from '@md/ui'
import { ResponsiveBar } from '@nivo/bar'
import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getSubscriptionComponentUsage } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

export interface UsageChartData {
  dataPoints: { windowStart: string; windowEnd: string; value: string; dimensions: Record<string, string> }[]
  periodStart: string
  periodEnd: string
}

interface UsageBarChartProps {
  subscriptionId: string
  metricId: string
}

/** Fetches via admin RPC and renders the chart */
export const UsageBarChart = ({ subscriptionId, metricId }: UsageBarChartProps) => {
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

  return <UsageBarChartDisplay data={usageQuery.data} />
}

/** Pure display component — accepts pre-fetched data */
export const UsageBarChartDisplay = ({ data }: { data: UsageChartData }) => {
  const { chartData, keys } = useMemo(() => {
    if (!data.dataPoints?.length) {
      return { chartData: [], keys: [] as string[] }
    }

    const dimensionKeys = new Set<string>()
    const grouped = new Map<string, Record<string, number>>()

    for (const point of data.dataPoints) {
      const dateKey = point.windowStart
      if (!grouped.has(dateKey)) {
        grouped.set(dateKey, {})
      }
      const entry = grouped.get(dateKey)!

      const dimEntries = Object.entries(point.dimensions)
      if (dimEntries.length === 0) {
        const key = 'usage'
        dimensionKeys.add(key)
        entry[key] = (entry[key] || 0) + Number(point.value)
      } else {
        const dimValue = dimEntries.map(([, v]) => v).join(' / ') || 'other'
        dimensionKeys.add(dimValue)
        entry[dimValue] = (entry[dimValue] || 0) + Number(point.value)
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

    const keys = Array.from(dimensionKeys)
    const chartData = allDays.map(day => {
      const values = grouped.get(day) ?? {}
      const filled: Record<string, number> = {}
      for (const k of keys) {
        filled[k] = values[k] ?? 0
      }
      return { date: formatShortDate(day), ...filled }
    })

    return { chartData, keys }
  }, [data])

  if (chartData.length === 0) {
    return (
      <div className="text-xs text-muted-foreground py-4 text-center">No usage data available</div>
    )
  }

  return (
    <div style={{ height: 180 }}>
      <ResponsiveBar
        data={chartData}
        keys={keys}
        indexBy="date"
        margin={{ top: 10, right: 10, bottom: 30, left: 50 }}
        padding={0.3}
        colors={{ scheme: 'paired' }}
        axisBottom={{
          tickSize: 0,
          tickPadding: 8,
          tickRotation: chartData.length > 14 ? -45 : 0,
          format: v => (chartData.length > 20 ? '' : v),
        }}
        axisLeft={{
          tickSize: 0,
          tickPadding: 8,
          format: v => formatCompact(Number(v)),
        }}
        enableLabel={false}
        enableGridY={true}
        theme={{
          text: { fill: 'hsl(var(--muted-foreground))', fontSize: 10 },
          grid: { line: { stroke: 'hsl(var(--border))', strokeWidth: 1 } },
          axis: { ticks: { text: { fill: 'hsl(var(--muted-foreground))' } } },
        }}
        tooltip={({ id, value, indexValue }) => (
          <div className="bg-popover text-popover-foreground border border-border rounded px-2 py-1 text-xs shadow-sm">
            <strong>{indexValue}</strong>: {id} = {formatCompact(value)}
          </div>
        )}
      />
    </div>
  )
}

function formatShortDate(dateStr: string): string {
  try {
    const d = new Date(dateStr)
    return `${d.getMonth() + 1}/${d.getDate()}`
  } catch {
    return dateStr
  }
}

function formatCompact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toFixed(n % 1 === 0 ? 0 : 2)
}
