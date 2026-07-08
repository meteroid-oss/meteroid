import { ResponsiveBar, type BarTooltipProps } from '@nivo/bar'
import { useCallback, useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getSubscriptionComponentUsage } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'

import { usePortalConfig } from './PortalThemeProvider'
import { hslToHex, type PortalThemeMode } from './theme'

import type { GetSubscriptionComponentUsageResponse } from '@/rpc/portal/subscription/v1/subscription_pb'

interface UsageChartProps {
  subscriptionId: string
  metricId: string
  startDate?: string
  endDate?: string
  /** When set, only data points matching these dimensions are shown (usage grouping filter). */
  groupByDimensions?: Record<string, string>
}

/** Portal-themed day-by-day usage chart. Fetches via the portal RPC and renders. */
export const UsageChart = ({
  subscriptionId,
  metricId,
  startDate,
  endDate,
  groupByDimensions,
}: UsageChartProps) => {
  const query = useQuery(
    getSubscriptionComponentUsage,
    { subscriptionId, metricId, startDate, endDate },
    { enabled: Boolean(subscriptionId) && Boolean(metricId) }
  )

  if (query.isLoading) {
    return (
      <div
        style={{
          height: 170,
          borderRadius: 'var(--mtp-r-sm)',
          background: 'var(--mtp-track)',
        }}
        className="mtp-pulse"
      />
    )
  }

  if (query.isError || !query.data?.dataPoints?.length) {
    return (
      <div style={{ padding: '20px 0', fontSize: 12, color: 'var(--mtp-text-3)', textAlign: 'center' }}>
        {query.isError ? 'Failed to load usage data' : 'No usage recorded yet this cycle.'}
      </div>
    )
  }

  return <UsageChartDisplay data={query.data} groupByDimensions={groupByDimensions} />
}

/** Pure display — shapes the daily windows into stacked bars themed with portal tokens. */
const UsageChartDisplay = ({
  data,
  groupByDimensions,
}: {
  data: GetSubscriptionComponentUsageResponse
  groupByDimensions?: Record<string, string>
}) => {
  const { accent, theme } = usePortalConfig()

  const { chartData, keys, colorMap } = useMemo(() => {
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
      if (!grouped.has(dateKey)) grouped.set(dateKey, {})
      const entry = grouped.get(dateKey)!

      // Remaining dimensions sorted by key for a consistent stacking order.
      const remainingDims = Object.entries(point.dimensions)
        .filter(([k]) => !groupByKeys.has(k))
        .sort(([a], [b]) => a.localeCompare(b))

      const key = remainingDims.length === 0 ? 'usage' : remainingDims.map(([, v]) => v).join(' / ')
      dimensionKeys.add(key)
      entry[key] = (entry[key] || 0) + Number(point.value)
    }

    // Fill the whole period so gaps render as empty days, not collapsed bars.
    const allDays: string[] = []
    if (data.periodStart && data.periodEnd) {
      const start = new Date(data.periodStart)
      const end = new Date(data.periodEnd)
      for (const d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
        allDays.push(d.toISOString().slice(0, 10))
      }
    } else {
      allDays.push(...grouped.keys())
    }

    const keys = Array.from(dimensionKeys).sort()
    const chartData = allDays.map(day => {
      const values = grouped.get(day) ?? {}
      const filled: Record<string, number | string> = {}
      for (const k of keys) filled[k] = values[k] ?? 0
      return { date: formatShortDate(day), rawDate: day, ...filled }
    })

    const palette = buildPalette(accent, theme, keys.length)
    const colorMap = new Map<string, string>()
    keys.forEach((k, i) => colorMap.set(k, palette[i]))

    return { chartData, keys, colorMap }
  }, [data, groupByDimensions, accent, theme])

  const barTooltip = useCallback(
    ({ data: barData }: BarTooltipProps<Record<string, number | string>>) => {
      const fullDate = formatFullDate(barData.rawDate as string)
      const entries = keys
        .map(k => ({ label: k, value: (barData[k] as number) ?? 0, color: colorMap.get(k)! }))
        .filter(e => e.value > 0)
      const total = entries.reduce((sum, e) => sum + e.value, 0)

      return (
        <div
          style={{
            background: 'var(--mtp-surface)',
            color: 'var(--mtp-text)',
            border: '1px solid var(--mtp-border-2)',
            borderRadius: 'var(--mtp-r-sm)',
            padding: '8px 10px',
            fontSize: 12,
            minWidth: 150,
            boxShadow: '0 6px 20px rgba(0,0,0,0.18)',
          }}
        >
          <div style={{ fontWeight: 600, marginBottom: 6 }}>{fullDate}</div>
          {entries.length === 0 ? (
            <div style={{ color: 'var(--mtp-text-3)' }}>No usage</div>
          ) : (
            entries.map(e => (
              <div
                key={e.label}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12,
                  padding: '1px 0',
                }}
              >
                <span style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--mtp-text-2)' }}>
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: '50%',
                      background: e.color,
                      flexShrink: 0,
                    }}
                  />
                  {e.label === 'usage' ? 'Usage' : e.label}
                </span>
                <span style={{ fontVariantNumeric: 'tabular-nums', fontWeight: 500 }}>
                  {formatCompact(e.value)}
                </span>
              </div>
            ))
          )}
          {entries.length > 1 && (
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 12,
                marginTop: 5,
                paddingTop: 5,
                borderTop: '1px solid var(--mtp-border)',
                fontWeight: 600,
              }}
            >
              <span>Total</span>
              <span style={{ fontVariantNumeric: 'tabular-nums' }}>{formatCompact(total)}</span>
            </div>
          )}
        </div>
      )
    },
    [keys, colorMap]
  )

  if (chartData.length === 0) {
    return (
      <div style={{ padding: '20px 0', fontSize: 12, color: 'var(--mtp-text-3)', textAlign: 'center' }}>
        No usage data available
      </div>
    )
  }

  const showLegend = keys.length > 1

  // Thin out x-axis labels so dense periods don't overlap.
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
          margin={{ top: 10, right: 8, bottom: 28, left: 44 }}
          padding={0.32}
          colors={bar => colorMap.get(bar.id as string) ?? 'var(--mtp-fill)'}
          borderRadius={2}
          axisBottom={{
            tickSize: 0,
            tickPadding: 8,
            format: v => (visibleTicks.has(v as string) ? (v as string) : ''),
          }}
          axisLeft={{
            tickSize: 0,
            tickPadding: 6,
            format: v => formatCompact(Number(v)),
          }}
          enableLabel={false}
          enableGridY
          gridYValues={4}
          tooltip={barTooltip}
          theme={{
            text: { fill: 'var(--mtp-text-3)', fontSize: 10 },
            grid: { line: { stroke: 'var(--mtp-border)', strokeWidth: 1 } },
            axis: { ticks: { text: { fill: 'var(--mtp-text-3)' } } },
          }}
        />
      </div>
      {showLegend && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px 12px', padding: '2px 2px 0' }}>
          {keys.map(k => (
            <span
              key={k}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                fontSize: 11,
                color: 'var(--mtp-text-3)',
              }}
            >
              <span
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: '50%',
                  background: colorMap.get(k),
                  flexShrink: 0,
                }}
              />
              {k === 'usage' ? 'Usage' : k}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

/** Brand-anchored series palette: the accent leads, extra series rotate hue harmoniously. */
const buildPalette = (accent: string, mode: PortalThemeMode, n: number): string[] => {
  if (n <= 1) return [accent]
  const [h, s] = hexToHsl(accent)
  const sat = Math.max(45, Math.min(72, s))
  const light = mode === 'dark' ? 64 : 50
  return Array.from({ length: n }, (_, i) =>
    i === 0 ? accent : hslToHex(`${(h + i * 47) % 360} ${sat}% ${light}%`)
  )
}

const hexToHsl = (hex: string): [number, number, number] => {
  let s = hex.replace('#', '')
  if (s.length === 3) s = s.split('').map(c => c + c).join('')
  const r = parseInt(s.slice(0, 2), 16) / 255
  const g = parseInt(s.slice(2, 4), 16) / 255
  const b = parseInt(s.slice(4, 6), 16) / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  const d = max - min
  if (d === 0) return [0, 0, l * 100]
  const sat = l > 0.5 ? d / (2 - max - min) : d / (max + min)
  let hue: number
  switch (max) {
    case r:
      hue = (g - b) / d + (g < b ? 6 : 0)
      break
    case g:
      hue = (b - r) / d + 2
      break
    default:
      hue = (r - g) / d + 4
  }
  return [hue * 60, sat * 100, l * 100]
}

const formatShortDate = (dateStr: string): string => {
  const d = new Date(dateStr + 'T00:00:00')
  if (Number.isNaN(d.getTime())) return dateStr
  return `${d.toLocaleString('en-US', { month: 'short' })} ${d.getDate()}`
}

const formatFullDate = (dateStr: string): string => {
  const d = new Date(dateStr + 'T00:00:00')
  if (Number.isNaN(d.getTime())) return dateStr
  return d.toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' })
}

const formatCompact = (n: number): string => {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toFixed(n % 1 === 0 ? 0 : 2)
}
