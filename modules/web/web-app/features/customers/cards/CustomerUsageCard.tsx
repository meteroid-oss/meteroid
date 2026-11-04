import { Card, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Skeleton } from '@md/ui'
import { format, subDays } from 'date-fns'
import { Gauge } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatUsage } from '@/lib/utils/numbers'
import { getCustomerUsage } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'

type RangePreset = '7d' | '30d' | '90d'

const RANGES: Record<RangePreset, { label: string; days: number }> = {
  '7d': { label: 'Last 7 days', days: 7 },
  '30d': { label: 'Last 30 days', days: 30 },
  '90d': { label: 'Last 90 days', days: 90 },
}

/** Friendly labels for the snake_case aggregation ops the server reports. */
const AGGREGATION_LABEL: Record<string, string> = {
  sum: 'sum',
  avg: 'average',
  min: 'min',
  max: 'peak',
  count: 'count',
  count_distinct: 'unique',
  latest: 'latest',
}

const toYmd = (d: Date) => format(d, 'yyyy-MM-dd')

/** Dependency-free inline sparkline (normalised polyline), fills width, inherits colour via currentColor. */
const Sparkline = ({ values, height = 22 }: { values: number[]; height?: number }) => {
  const w = 96
  const h = height
  if (values.length < 2) {
    return <div style={{ height: h }} />
  }
  let min = values[0]
  let max = values[0]
  for (const v of values) {
    if (v < min) min = v
    if (v > max) max = v
  }
  const span = max - min || 1
  const points = values
    .map((v, i) => {
      const x = (i / (values.length - 1)) * w
      const y = h - ((v - min) / span) * (h - 2) - 1
      return `${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')
  return (
    <svg
      width="100%"
      height={h}
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      aria-hidden
    >
      <polyline
        points={points}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  )
}

/**
 * Per-customer usage: every tenant billable metric's usage for this customer in one batched
 * call (the server fans out the per-metric aggregates and returns partial results if one
 * errors). A ranked table of the whole-range total (in the metric's op) plus a daily trend.
 */
export const CustomerUsageCard = ({ customerId }: { customerId: string }) => {
  const basePath = useBasePath()
  const [range, setRange] = useState<RangePreset>('30d')

  const { from, to } = useMemo(() => {
    const now = new Date()
    return { from: toYmd(subDays(now, RANGES[range].days)), to: toYmd(now) }
  }, [range])

  const query = useQuery(getCustomerUsage, { customerId, from, to })

  // What the customer actually uses this period, ranked. Defined-but-unused meters are
  // omitted — a customer typically touches a handful even when the tenant meters on many.
  const rows = useMemo(
    () =>
      [...(query.data?.metrics ?? [])].filter(m => m.total > 0).sort((a, b) => b.total - a.total),
    [query.data?.metrics]
  )
  const evaluated = query.data?.metrics?.length ?? 0
  const totalMetrics = query.data?.totalCount ?? evaluated
  // Fewer metrics than the tenant has means the set was capped or a metric's query failed;
  // surface it so an unread metric never masquerades as "no usage".
  const partial = totalMetrics > evaluated

  if (query.data && query.data.totalCount === 0) {
    return (
      <Card className="overflow-hidden">
        <PanelEmptyState
          icon={Gauge}
          title="No usage metrics yet"
          description="Define billable metrics in the product catalog to track this customer's usage."
        />
      </Card>
    )
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <div className="text-[13px] text-muted-foreground">
          {[
            rows.length > 0 ? `${rows.length} active` : null,
            partial ? `${evaluated} of ${totalMetrics} metrics read` : null,
          ]
            .filter(Boolean)
            .join(' · ')}
        </div>
        <Select value={range} onValueChange={v => setRange(v as RangePreset)}>
          <SelectTrigger className="w-[160px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(Object.keys(RANGES) as RangePreset[]).map(key => (
              <SelectItem key={key} value={key}>
                {RANGES[key].label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <Card className="overflow-hidden">
        <div className="grid grid-cols-[1fr_auto_auto] items-center gap-4 px-4 py-2 border-b border-border text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
          <div>Metric</div>
          <div className="text-right">Usage</div>
          <div className="w-24 text-right">Trend</div>
        </div>

        {query.isLoading ? (
          <div className="divide-y divide-border">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="px-4 py-3">
                <Skeleton height={20} />
              </div>
            ))}
          </div>
        ) : rows.length === 0 ? (
          <div className="px-4 py-10 text-center text-sm text-muted-foreground">
            {query.error ? 'Usage is temporarily unavailable.' : 'No usage recorded in this period.'}
          </div>
        ) : (
          <div className="divide-y divide-border">
            {rows.map(m => (
              <div
                key={m.metricId}
                className="grid grid-cols-[1fr_auto_auto] items-center gap-4 px-4 py-2.5 text-[13px] hover:bg-muted/40"
              >
                <div className="min-w-0">
                  <Link
                    to={`${basePath}/metrics/${m.metricId}`}
                    className="font-medium text-foreground hover:text-brand hover:underline truncate block"
                    title={m.label}
                  >
                    {m.label}
                  </Link>
                  <div className="text-[11px] text-muted-foreground">
                    {AGGREGATION_LABEL[m.aggregation] ?? m.aggregation}
                  </div>
                </div>
                <div className="text-right tabular-nums font-medium">{formatUsage(m.total)}</div>
                <div className="w-24 flex justify-end text-brand">
                  <Sparkline values={m.trend.map(p => p.value)} />
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  )
}
