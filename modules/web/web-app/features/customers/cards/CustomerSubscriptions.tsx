import { skipToken } from '@connectrpc/connect-query'
import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { FeatureUsageList, byUsage } from '@/features/subscriptions/FeatureUsageList'
import { SubscriptionStatusBadge } from '@/features/subscriptions/SubscriptionStatusBadge'
import { useSubscriptionUsage } from '@/features/subscriptions/SubscriptionUsageSection'
import { formatUsageFilter } from '@/features/subscriptions/UsageFilterFields'
import { issueDay, nextInvoiceAmount } from '@/features/subscriptions/subscriptionOverview'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatMoney } from '@/lib/utils/format'
import { Subscription, SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getUpcomingInvoice,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

const PAGE_SIZE = 5
const TOP_METERS = 3

const isLive = (s: Subscription) =>
  (s.status === SubscriptionStatus.ACTIVE || s.status === SubscriptionStatus.TRIALING) &&
  !!s.currentPeriodStart

/**
 * The customer's subscriptions, one row each: plan and version, status, MRR, where the cycle
 * stands, the next invoice, and the most used limited features. The details are on the
 * subscription page.
 */
export const CustomerSubscriptions = ({ customerId }: { customerId: string }) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: PAGE_SIZE,
  })
  const query = useQuery(listSubscriptions, {
    pagination: { perPage: pagination.pageSize, page: pagination.pageIndex },
    customerId,
    status: [],
  })

  if (query.isLoading) {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton height={56} />
        <Skeleton height={56} />
      </div>
    )
  }

  const subscriptions = query.data?.subscriptions ?? []
  const total = Number(query.data?.paginationMeta?.totalItems ?? 0)

  if (subscriptions.length === 0) {
    return <p className="text-sm text-muted-foreground">No subscription yet.</p>
  }

  return (
    <div className="flex flex-col gap-2">
      <ul className="divide-y divide-border rounded-md border border-border">
        {subscriptions.map(s => (
          <SubscriptionRow key={s.id} subscription={s} />
        ))}
      </ul>
      <Pagination
        pagination={pagination}
        setPagination={setPagination}
        totalCount={total}
        isLoading={query.isLoading}
      />
    </div>
  )
}

const SubscriptionRow = ({ subscription: s }: { subscription: Subscription }) => {
  const basePath = useBasePath()
  const live = isLive(s)
  const usage = useSubscriptionUsage(live ? s.id : undefined)
  const next = useQuery(getUpcomingInvoice, live ? { subscriptionId: s.id } : skipToken, {
    staleTime: 60_000,
  })
  const meters = byUsage(usage.data?.usage ?? [])
    .filter(u => u.limit)
    .slice(0, TOP_METERS)
  const invoice = next.data?.invoice

  return (
    <li>
      <Link
        to={`${basePath}/subscriptions/${s.id}`}
        className="flex flex-col gap-2 px-3 py-2.5 hover:bg-muted/30"
      >
        <div className="flex items-center gap-2 text-sm">
          <span className="truncate font-medium text-foreground">
            {s.planName} v{s.version}
          </span>
          {s.status !== SubscriptionStatus.ACTIVE && <SubscriptionStatusBadge status={s.status} />}
          <span className="ml-auto shrink-0 tabular-nums text-foreground">
            {formatMoney(Number(s.mrrCents), s.currency)}
            <span className="text-xs text-muted-foreground"> MRR</span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        </div>
        {s.usageFilter && (
          <span className="text-xs text-muted-foreground">
            Usage filter <span className="font-mono">{formatUsageFilter(s.usageFilter)}</span>
          </span>
        )}
        {live && (
          <div className="flex flex-wrap items-end gap-x-4 gap-y-1">
            <span className="text-xs text-muted-foreground">
              {invoice
                ? `Next invoice ${nextInvoiceAmount(invoice, minor => formatMoney(Number(minor), invoice.currency))} on ${formatShortDay(issueDay(invoice))}`
                : next.isLoading
                  ? 'Computing the next invoice…'
                  : next.isError
                    ? 'Next invoice: open the subscription'
                    : undefined}
            </span>
          </div>
        )}
        {meters.length > 0 && (
          <FeatureUsageList
            usage={meters}
            elapsedPct={usage.data?.cycle?.elapsedPct}
            details={false}
          />
        )}
      </Link>
    </li>
  )
}
