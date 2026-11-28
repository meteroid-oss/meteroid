import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { format, subDays } from 'date-fns'
import { Gauge, TriangleAlert } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router'

import { Sparkline } from '@/components/charts/Sparkline'
import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { ContainedTableSection } from '@/features/customers/cards/ContainedTableSection'
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

/**
 * Per-customer usage: every tenant billable metric's usage for this customer in one batched
 * call (the server fans out the per-metric aggregates and returns partial results if one
 * errors). A ranked table of the whole-range total (in the metric's op) plus a daily trend, in
 * the customer page's contained table.
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
      <SectionEmptyState
        className="bg-card"
        icon={<Gauge />}
        title="No usage metrics yet"
        action={
          <Button size="sm" variant="secondary" asChild>
            <Link to={`${basePath}/metrics`}>Go to metrics</Link>
          </Button>
        }
      >
        Define billable metrics in the product catalog to track this customer&apos;s usage.
      </SectionEmptyState>
    )
  }

  const summary = [
    rows.length > 0 ? `${rows.length} active` : null,
    partial ? `${evaluated} of ${totalMetrics} metrics read` : null,
  ]
    .filter(Boolean)
    .join(' · ')

  return (
    <ContainedTableSection
      title="Usage"
      headerExtra={
        <>
          {summary && <span className="text-[13px] text-muted-foreground">{summary}</span>}
          <Select value={range} onValueChange={v => setRange(v as RangePreset)}>
            <SelectTrigger className="h-7 w-[140px] text-[13px]">
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
        </>
      }
    >
      {/* Laid out as the invoices table beside it (24px in on the left, 32px column headers),
          with taller rows for the metric's name, its aggregation and the trend. */}
      <Table className="[&_td:first-child]:pl-6 [&_td:last-child]:pr-6 [&_td]:px-2.5 [&_th:first-child]:pl-6 [&_th:last-child]:pr-6 [&_th]:!h-8 [&_th]:px-2.5 [&_th]:text-[13px] [&_th]:font-medium [&_th]:text-foreground/60">
        <TableHeader className="border-t-0 bg-transparent">
          <TableRow className="hover:bg-transparent">
            <TableHead>Metric</TableHead>
            <TableHead className="text-right">Usage</TableHead>
            <TableHead className="w-32 text-right">Trend</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {query.isLoading ? (
            Array.from({ length: 3 }, (_, i) => (
              <TableRow key={i} className="h-[60px] hover:bg-transparent">
                <TableCell colSpan={3}>
                  <Skeleton height={20} />
                </TableCell>
              </TableRow>
            ))
          ) : rows.length === 0 ? (
            <TableRow className="hover:bg-transparent">
              <TableCell colSpan={3} className="p-0">
                {query.error ? (
                  <SectionEmptyState
                    framed={false}
                    icon={<TriangleAlert />}
                    title="Usage is temporarily unavailable"
                  >
                    The usage for this customer could not be read. Try again in a moment.
                  </SectionEmptyState>
                ) : (
                  <SectionEmptyState
                    framed={false}
                    icon={<Gauge />}
                    title={`No usage in the ${RANGES[range].label.toLowerCase()}`}
                  >
                    Events this customer reports for your metrics show here. Try a longer period.
                  </SectionEmptyState>
                )}
              </TableCell>
            </TableRow>
          ) : (
            rows.map(m => (
              <TableRow
                key={m.metricId}
                className="h-[60px] text-[13px] hover:bg-foreground/[0.04]"
              >
                <TableCell className="max-w-0">
                  <Link
                    to={`${basePath}/metrics/${m.metricId}`}
                    className="block truncate font-medium text-foreground hover:underline"
                    title={m.label}
                  >
                    {m.label}
                  </Link>
                  <div className="text-[11px] text-muted-foreground">
                    {AGGREGATION_LABEL[m.aggregation] ?? m.aggregation}
                  </div>
                </TableCell>
                <TableCell className="whitespace-nowrap text-right font-medium tabular-nums">
                  {formatUsage(m.total)}
                </TableCell>
                <TableCell className="w-32">
                  <div className="ml-auto w-24 text-brand">
                    <Sparkline values={m.trend.map(p => p.value)} />
                  </div>
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </ContainedTableSection>
  )
}
