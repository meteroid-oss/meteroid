import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Switch,
} from '@md/ui'
import { ExternalLink, Users } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import { useModule } from '@/edition'
import { isAdditiveAggregation } from '@/features/productCatalog/metrics/aggregation'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { formatUsage } from '@/lib/utils/numbers'
import { queryMetricUsage } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'
import {
  QueryMetricUsageRequest_Breakdown,
  QueryMetricUsageRequest_Grain,
} from '@/rpc/api/eventinsights/v1/eventinsights_pb'

import type { Aggregation_AggregationType } from '@/rpc/api/billablemetrics/v1/models_pb'

type RangePreset = '7d' | '30d' | '90d'

const RANGES: Record<RangePreset, { label: string; days: number }> = {
  '7d': { label: 'Last 7 days', days: 7 },
  '30d': { label: 'Last 30 days', days: 30 },
  '90d': { label: 'Last 90 days', days: 90 },
}

/** The dimension key the server labels a customer breakdown's points with. */
const CUSTOMER_DIMENSION = 'customer'

/** The overflow bucket the server folds the long tail into (metric_core OTHER_LABEL). */
const OTHER_LABEL = 'Other'

/** How many customers to rank; the rest land in the server's "Other" bucket. */
const TOP_N = 8

interface CustomerUsage {
  name: string
  value: number
  isOther: boolean
  /** The customer's id, for deep-linking the row (absent for the "Other" bucket). */
  customerId?: string
}

interface MetricCustomersCardProps {
  metricId: string
  aggregationType: Aggregation_AggregationType
}

/**
 * "How customers are using this metric": the saved metric's whole-range usage per customer,
 * metered as billing reads it. Ranked by usage, with the long tail folded into "Other"
 * server-side (sum/count only; other aggregations drop it).
 */
export const MetricCustomersCard = ({ metricId, aggregationType }: MetricCustomersCardProps) => {
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const hasIntelligence = useModule('intelligence') !== undefined
  const showShare = isAdditiveAggregation(aggregationType)
  const [range, setRange] = useState<RangePreset>('30d')
  // The long-tail "Other" bucket can dwarf the named customers and squash their bars,
  // so it's hidden by default; the toggle folds it back in when the split is wanted.
  const [showOther, setShowOther] = useState(false)

  const { from, to } = useMemo(() => {
    const now = new Date()
    const start = new Date(now)
    start.setUTCDate(start.getUTCDate() - RANGES[range].days)
    return { from: start, to: now }
  }, [range])

  // One whole-range aggregate per customer, computed with the metric's op, so the ranking
  // is correct for max/latest/count_distinct — not summed per day.
  const usageQuery = useQuery(queryMetricUsage, {
    metric: { case: 'metricId', value: metricId },
    from: timestampFromDate(from),
    to: timestampFromDate(to),
    grain: QueryMetricUsageRequest_Grain.TOTAL,
    breakdown: QueryMetricUsageRequest_Breakdown.CUSTOMER,
    topN: TOP_N,
  })

  // Rank desc, with "Other" pinned last so the long tail reads as a footer.
  const rows: CustomerUsage[] = useMemo(
    () =>
      (usageQuery.data?.points ?? [])
        .map(p => ({
          name: p.other ? OTHER_LABEL : (p.dimensions[CUSTOMER_DIMENSION] ?? p.customerId),
          value: p.value,
          customerId: p.customerId || undefined,
          isOther: p.other,
        }))
        .sort((a, b) => {
          if (a.isOther !== b.isOther) return a.isOther ? 1 : -1
          return b.value - a.value
        }),
    [usageQuery.data?.points]
  )

  const hasOther = useMemo(() => rows.some(r => r.isOther), [rows])
  const visibleRows = useMemo(
    () => (showOther ? rows : rows.filter(r => !r.isOther)),
    [rows, showOther]
  )

  // Shares and bar widths are relative to what's displayed, so dropping "Other"
  // rescales the named customers to fill the chart instead of leaving them tiny.
  const total = useMemo(() => visibleRows.reduce((acc, r) => acc + r.value, 0), [visibleRows])
  const max = useMemo(() => visibleRows.reduce((acc, r) => Math.max(acc, r.value), 0), [visibleRows])

  const isLoading = usageQuery.isLoading

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div className="flex flex-col gap-1">
          <div className="text-xs font-semibold text-muted-foreground">Top customers</div>
        </div>
        <div className="flex items-center gap-2">
          {hasOther && (
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer select-none">
              <Switch checked={showOther} onCheckedChange={setShowOther} />
              Show Other
            </label>
          )}
          <Select value={range} onValueChange={v => setRange(v as RangePreset)}>
            <SelectTrigger className="w-[160px] h-7 text-xs">
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
          {hasIntelligence && !isExpress && (
            <Button variant="secondary" size="sm" className="h-7 mt-0 text-xs" asChild>
              <Link
                to={`${basePath}/analytics/reports/insights/bm:${metricId}?split=${CUSTOMER_DIMENSION}`}
              >
                <ExternalLink size={14} className="mr-1" />
                Explore
              </Link>
            </Button>
          )}
        </div>
      </div>

      {isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} height={28} />
          ))}
        </div>
      ) : visibleRows.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-32 text-muted-foreground gap-2">
          <Users size={20} className="opacity-60" />
          <div className="text-sm">No customer usage in this period</div>
        </div>
      ) : (
        <div className="space-y-2.5">
          {visibleRows.map(row => {
            const share = total > 0 ? (row.value / total) * 100 : 0
            const width = max > 0 ? (row.value / max) * 100 : 0
            return (
              <div key={row.customerId ?? row.name} className="flex items-center gap-3 text-[13px]">
                <div
                  className={`w-40 shrink-0 truncate ${
                    row.isOther ? 'text-muted-foreground italic' : 'text-foreground font-medium'
                  }`}
                  title={row.name}
                >
                  {row.customerId ? (
                    <Link
                      to={`${basePath}/customers/${row.customerId}`}
                      className="hover:text-brand hover:underline"
                    >
                      {row.name}
                    </Link>
                  ) : (
                    row.name
                  )}
                </div>
                <div className="flex-1 h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className={`h-full rounded-full ${row.isOther ? 'bg-muted-foreground/40' : 'bg-brand'}`}
                    style={{ width: `${Math.max(width, 2)}%` }}
                  />
                </div>
                <div className="w-24 shrink-0 text-right tabular-nums font-medium">
                  {formatUsage(row.value)}
                </div>
                {showShare && (
                  <div className="w-12 shrink-0 text-right tabular-nums text-muted-foreground">
                    {share.toFixed(0)}%
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
