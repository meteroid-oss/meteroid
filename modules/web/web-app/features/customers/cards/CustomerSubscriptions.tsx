import { skipToken } from '@connectrpc/connect-query'
import { Button, Skeleton, cn } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ChevronRight, Plus, Repeat } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router'

import { raisedList, raisedListLinkRow } from '@/components/RaisedList'
import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { useResourceChips } from '@/features/resources/hooks'
import { FeatureUsageList, byUsage } from '@/features/subscriptions/FeatureUsageList'
import { SubscriptionStatusBadge } from '@/features/subscriptions/SubscriptionStatusBadge'
import { useSubscriptionUsage } from '@/features/subscriptions/SubscriptionUsageSection'
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
export const CustomerSubscriptions = ({
  customerId,
  onCreate,
}: {
  customerId: string
  /** Starts a subscription for this customer; the empty state offers it when set. */
  onCreate?: () => void
}) => {
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
    return (
      <SectionEmptyState
        className="bg-card dark:bg-foreground/[0.035]"
        icon={<Repeat />}
        title="No subscriptions yet"
        action={
          onCreate && (
            <Button size="sm" variant="secondary" hasIcon onClick={onCreate}>
              <Plus size={14} /> New subscription
            </Button>
          )
        }
      >
        Subscribe this customer to a plan to start billing them. Their subscriptions show here.
      </SectionEmptyState>
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <ul className={raisedList}>
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
  const chips = useResourceChips(s.customerId)
  const basePath = useBasePath()
  const live = isLive(s)
  const usage = useSubscriptionUsage(live ? s.id : undefined)
  const next = useQuery(getUpcomingInvoice, live ? { subscriptionId: s.id } : skipToken, {
    staleTime: 60_000,
    // As the summary's next-charge tile, which shares this query: one retry, not three.
    retry: 1,
  })
  const meters = byUsage(usage.data?.usage ?? [])
    .filter(u => u.limit)
    .slice(0, TOP_METERS)
  const invoice = next.data?.invoice

  return (
    <li>
      <Link
        to={`${basePath}/subscriptions/${s.id}`}
        className={cn('flex items-center gap-4', raisedListLinkRow)}
      >
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <div className="flex min-w-0 items-center gap-2 text-sm">
            <span className="truncate font-medium text-foreground">
              {s.planName} v{s.version}
            </span>
            {s.status !== SubscriptionStatus.ACTIVE && (
              <SubscriptionStatusBadge status={s.status} />
            )}
          </div>
          {s.resourceId && (
            <span className="text-xs text-muted-foreground">
              For {chips.get(s.resourceId) ?? 'a resource'}
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
              customerId={s.customerId}
              elapsedPct={usage.data?.cycle?.elapsedPct}
              details={false}
              groupByProduct={false}
            />
          )}
        </div>
        {/* The MRR and the chevron, centred on the row's height. */}
        <div className="flex shrink-0 items-center gap-2 text-sm">
          <span className="tabular-nums text-foreground">
            {formatMoney(Number(s.mrrCents), s.currency)}
            <span className="text-xs text-muted-foreground"> MRR</span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        </div>
      </Link>
    </li>
  )
}
