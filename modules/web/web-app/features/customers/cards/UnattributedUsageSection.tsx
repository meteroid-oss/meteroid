import {
  Button,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { AlertTriangle, ChevronLeft, ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { formatMetricQuantity, formatRawQuantity } from '@/features/billing/metrics/units'
import { useResourceChips } from '@/features/resources/hooks'
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

const subscriptionLabel = (subscription: Subscription, chips: Map<string, string>) =>
  subscription.resourceId
    ? `${subscription.planName} (${chips.get(subscription.resourceId) ?? 'a resource'})`
    : subscription.planName

export const UnattributedUsageSection = ({ customerId }: { customerId: string }) => {
  const basePath = useBasePath()
  const chips = useResourceChips(customerId)
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
      <section className="flex flex-col gap-4">
        <h3 className="text-base font-medium">Unattributed usage</h3>
        <p className="text-sm text-destructive">Unattributed usage could not be loaded.</p>
      </section>
    )
  }

  if (recentRows.length === 0 && selected === undefined) return null

  const usage = monthQuery.data?.usage ?? []

  return (
    <section id="unattributed-usage" className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-base font-medium">Unattributed usage</h3>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
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
            size="icon"
            className="h-7 w-7"
            aria-label="Next month"
            disabled={month >= thisMonth}
            onClick={() => setSelected(monthStart(new Date(month), 1))}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>
      <div className="overflow-x-auto rounded-md border border-border">
        {monthQuery.isLoading ? (
          <div className="p-4">
            <Skeleton className="h-16 w-full" />
          </div>
        ) : usage.length === 0 ? (
          <PanelEmptyState quiet title={`None in ${monthLabel(month)}`} className="py-6" />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Metric</TableHead>
                <TableHead>Period</TableHead>
                <TableHead>Priced by</TableHead>
                <TableHead className="text-right">Quantity</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {usage.map(row => (
                <TableRow key={`${row.metricId}-${row.from}`}>
                  <TableCell className="text-sm font-medium">{row.metricName}</TableCell>
                  <TableCell className="text-[13px] text-muted-foreground">
                    {periodLabel(row.from, row.until)}
                  </TableCell>
                  <TableCell className="text-[13px]">
                    {row.subscriptionIds.length === 0 ? (
                      <span className="text-muted-foreground">—</span>
                    ) : (
                      <div className="flex flex-col gap-0.5">
                        {row.subscriptionIds.map(id => {
                          const subscription = subscriptionsById.get(id)
                          return (
                            <Link
                              key={id}
                              to={`${basePath}/subscriptions/${id}`}
                              className="font-medium text-brand hover:underline"
                            >
                              {subscription
                                ? subscriptionLabel(subscription, chips)
                                : 'Subscription'}
                            </Link>
                          )
                        })}
                      </div>
                    )}
                  </TableCell>
                  <TableCell className="text-right text-[13px] tabular-nums">
                    {quantityLabel(row)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>
    </section>
  )
}

/** Only on the customer page: usage no subscription bills this month, linking to the section. */
export const UnattributedBanner = ({ customerId }: { customerId: string }) => {
  const now = useMemo(() => new Date(), [])
  const usage =
    useQuery(listUnattributedUsage, { customerId, from: monthStart(now), to: monthStart(now, 1) })
      .data?.usage ?? []
  if (!usage.length) return null
  return (
    <a
      href="#unattributed-usage"
      className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs hover:bg-warning/15"
    >
      <AlertTriangle size={14} className="mt-px shrink-0 text-warning" />
      <span>
        Unattributed usage this month: {[...new Set(usage.map(u => u.metricName))].join(', ')}
      </span>
    </a>
  )
}
