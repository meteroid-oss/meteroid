import { Skeleton } from '@md/ui'

import { HatchedBar, formatApprox } from '@/features/subscriptions/PeriodUsageBar'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import { formatDateRange, formatShortDay } from '@/features/subscriptions/utils/dates'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrency } from '@/lib/utils/numbers'
import { getSubscriptionMinimumProgress } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

const PROGRESS_STALE_MS = 60_000

/** In-scope spend so far against the period's minimum, projected to the period end. */
export const SubscriptionMinimumProgress = ({ subscriptionId }: { subscriptionId: string }) => {
  const query = useQuery(
    getSubscriptionMinimumProgress,
    { subscriptionId },
    { staleTime: PROGRESS_STALE_MS }
  )
  if (query.isLoading) {
    return (
      <div className="mb-6 rounded-lg border border-border bg-card p-4">
        <Skeleton height={16} width={200} className="mb-3" />
        <Skeleton height={12} />
      </div>
    )
  }
  const progress = query.data?.progress
  if (!progress) return null

  const currency = progress.currency
  const money = (minor: bigint) => formatCurrency(minor, currency)
  const denomination = { currency }
  const pct = (minor: bigint) =>
    progress.minimum > 0n
      ? Math.min(100, Math.max(0, (Number(minor) / Number(progress.minimum)) * 100))
      : 0
  const shortfall = progress.projectedShortfall > 0n
  const tone = shortfall ? 'warning' : 'brand'
  const projectedLabel = `Projected by ${formatShortDay(progress.periodEnd)} ≈ ${formatApprox(progress.projected, denomination)}`
  const outcome = shortfall
    ? `Projected shortfall ≈ ${formatApprox(progress.projectedShortfall, denomination)}`
    : 'Minimum projected to be met'
  const scope =
    progress.scopeProductNames.length > 0 ? progress.scopeProductNames.join(', ') : undefined

  const updated = new Date(query.dataUpdatedAt).toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })

  return (
    <SubscriptionSectionCard
      title={`Minimum commitment · ${formatDateRange(progress.periodStart, progress.periodEnd)}`}
      subtitle={scope}
      aside={
        <span className={shortfall ? 'text-sm text-warning' : 'text-sm text-muted-foreground'}>
          {outcome}
        </span>
      }
    >
      <div className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-xl font-semibold tabular-nums text-foreground">
            {money(progress.countedToDate)}
          </span>
          <span className="text-sm text-muted-foreground">of {money(progress.minimum)}</span>
        </div>
        <HatchedBar
          usedPct={pct(progress.countedToDate)}
          projectedPct={pct(progress.projected)}
          tone={tone}
          label={`${money(progress.countedToDate)} of ${money(progress.minimum)} minimum used; ${projectedLabel.toLowerCase()}`}
        />
        <span className="text-xs text-muted-foreground tabular-nums">
          So far {money(progress.countedToDate)} · {projectedLabel} · updated {updated}
        </span>
      </div>
    </SubscriptionSectionCard>
  )
}
