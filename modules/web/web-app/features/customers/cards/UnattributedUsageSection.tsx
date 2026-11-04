import { Button, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@md/ui'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import { formatMetricQuantity, formatRawQuantity } from '@/features/billing/metrics/units'
import { formatUsageFilter } from '@/features/subscriptions/UsageFilterFields'
import { formatDateRange } from '@/features/subscriptions/utils/dates'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import {
  listSubscriptions,
  listUnattributedUsage,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import type { UnattributedUsage } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

// Periods are half-open: show the last day that still holds usage, not the exclusive end.
const lastIncludedDay = (end: string) => new Date(Date.parse(end) - 1).toISOString()

const periodLabel = (from: string, end: string) => formatDateRange(from, lastIncludedDay(end))

const quantityLabel = (usage: UnattributedUsage) =>
  usage.unitLabel
    ? formatMetricQuantity(usage.quantity, {
        singular: usage.unitLabel.singular,
        plural: usage.unitLabel.plural,
      })
    : formatRawQuantity(usage.quantity, undefined)

/** The first day (UTC) of the month `offset` months from `month`, as the API's date. */
export const monthStart = (month: Date, offset = 0) =>
  new Date(Date.UTC(month.getUTCFullYear(), month.getUTCMonth() + offset, 1))
    .toISOString()
    .slice(0, 10)

const monthLabel = (start: string) =>
  new Date(start).toLocaleDateString('en-US', { timeZone: 'UTC', month: 'long', year: 'numeric' })

const subscriptionLabel = (subscription: Subscription) =>
  subscription.usageFilter
    ? `${subscription.planName} (${formatUsageFilter(subscription.usageFilter)})`
    : subscription.planName

export const UnattributedUsageSection = ({ customerId }: { customerId: string }) => {
  const basePath = useBasePath()
  const now = useMemo(() => new Date(), [])
  const thisMonth = monthStart(now)
  // Shown when the last twelve months hold any (the server reads up to 366 days at once), on
  // the latest month that does; earlier months stay reachable from the selector.
  const recentQuery = useQuery(listUnattributedUsage, {
    customerId,
    from: monthStart(now, -11),
    to: monthStart(now, 1),
  })
  const [selected, setSelected] = useState<string | undefined>(undefined)
  const recentRows = recentQuery.data?.usage ?? []
  const latest = recentRows.reduce<number | undefined>(
    (max, row) => Math.max(max ?? 0, Date.parse(row.until) - 1),
    undefined
  )
  const month = selected ?? (latest !== undefined ? monthStart(new Date(latest)) : thisMonth)
  const monthQuery = useQuery(listUnattributedUsage, {
    customerId,
    from: month,
    to: monthStart(new Date(month), 1),
  })
  const subscriptionsQuery = useQuery(listSubscriptions, {
    customerId,
    pagination: { perPage: 100, page: 0 },
  })
  const subscriptionsById = useMemo(
    () => new Map((subscriptionsQuery.data?.subscriptions ?? []).map(s => [s.id, s])),
    [subscriptionsQuery.data]
  )

  if (recentQuery.isLoading) return null

  if (recentQuery.isError || monthQuery.isError) {
    return (
      <section className="flex flex-col gap-2">
        <h3 className="text-lg font-medium">Unattributed usage</h3>
        <p className="text-sm text-muted-foreground">The unattributed usage could not be loaded.</p>
      </section>
    )
  }

  if (recentRows.length === 0 && selected === undefined) return null

  const usage = monthQuery.data?.usage ?? []

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-lg font-medium">Unattributed usage</h3>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            aria-label="Previous month"
            onClick={() => setSelected(monthStart(new Date(month), -1))}
          >
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <span className="w-28 text-center text-xs text-muted-foreground">
            {monthLabel(month)}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            aria-label="Next month"
            disabled={month >= thisMonth}
            onClick={() => setSelected(monthStart(new Date(month), 1))}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>
      <p className="text-sm text-muted-foreground">
        Usage that matches no subscription&apos;s usage filter while every subscription pricing the
        metric has one. It is kept and billed to nobody. A new usage filter on an active
        subscription pricing the metric bills what is not invoiced yet; usage already invoiced
        stays unattributed.
      </p>
      {monthQuery.isLoading ? null : usage.length === 0 ? (
        <p className="text-sm text-muted-foreground">No unattributed usage in {monthLabel(month)}.</p>
      ) : (
        <div className="overflow-x-auto rounded-md border border-border">
          <Table className="text-xs">
            <TableHeader>
              <TableRow>
                <TableHead className="text-xs">Metric</TableHead>
                <TableHead className="text-xs">Period</TableHead>
                <TableHead className="text-xs">Priced by</TableHead>
                <TableHead className="text-right text-xs">Quantity</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {usage.map(row => (
                <TableRow key={`${row.metricId}-${row.from}`}>
                  <TableCell className="font-medium text-foreground">{row.metricName}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {periodLabel(row.from, row.until)}
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-col gap-0.5">
                      {row.subscriptionIds.map(id => {
                        const subscription = subscriptionsById.get(id)
                        return (
                          <Link
                            key={id}
                            to={`${basePath}/subscriptions/${id}`}
                            className="text-brand hover:underline"
                          >
                            {subscription ? subscriptionLabel(subscription) : 'Subscription'}
                          </Link>
                        )
                      })}
                    </div>
                  </TableCell>
                  <TableCell className="text-right tabular-nums">{quantityLabel(row)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  )
}
