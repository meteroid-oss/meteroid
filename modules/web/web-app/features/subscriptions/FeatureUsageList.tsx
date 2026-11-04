import { format } from 'date-fns'

import { UsageMeter } from '@/features/subscriptions/UsageMeter'
import { compactQuantity, formatQuantity, parseInstant } from '@/lib/utils/format'

import type { SubscriptionFeatureUsage } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

const RESET_LABEL: Record<string, string> = {
  calendar: 'calendar period',
  fixed_window: 'fixed window',
  sliding_window: 'sliding window',
  never: 'lifetime',
}

/**
 * What a limit does past it. A free trial bills no usage (the overage flag still reports usage
 * past the limit), so during one the note says when overage starts to be billed.
 */
export const limitBehaviorNote = (
  u: Pick<SubscriptionFeatureUsage, 'overageBehavior' | 'gracePct'>,
  freeTrialEnd?: Date
) => {
  if (u.overageBehavior === 'block') {
    return u.gracePct ? `Hard limit (+${u.gracePct}% grace)` : 'Hard limit'
  }
  if (u.overageBehavior !== 'allow') return undefined
  return freeTrialEnd
    ? `Soft limit · trial usage is not billed, overage is billed from ${format(freeTrialEnd, 'd MMM')}`
    : 'Soft limit, overage billed'
}

/** Most used first: the features closest to (or past) their limit lead. */
export const byUsage = (usage: SubscriptionFeatureUsage[]) =>
  [...usage].sort((a, b) => (b.pct ?? -1) - (a.pct ?? -1))

/**
 * Usage of limited features against their limits: a meter per feature with the projection, the
 * reset rule of limits not tied to the billing cycle, and hard limits.
 */
export const FeatureUsageList = ({
  usage,
  elapsedPct,
  details = true,
  freeTrialEnd,
}: {
  usage: SubscriptionFeatureUsage[]
  /** The cycle's elapsed share, marked on cycle-bound meters. */
  elapsedPct?: number
  /** Projection, reset rule and hard limit under each meter. */
  details?: boolean
  /** Set while the subscription is in a free trial: when it ends. */
  freeTrialEnd?: Date
}) => (
  <ul className="flex flex-col gap-3">
    {usage.map(u => (
      <FeatureUsageItem
        key={u.featureId}
        usage={u}
        elapsedPct={elapsedPct}
        details={details}
        freeTrialEnd={freeTrialEnd}
      />
    ))}
  </ul>
)

const FeatureUsageItem = ({
  usage: u,
  elapsedPct,
  details,
  freeTrialEnd,
}: {
  usage: SubscriptionFeatureUsage
  elapsedPct?: number
  details: boolean
  freeTrialEnd?: Date
}) => {
  const cycleBound = u.reset === 'billing_cycle'
  const resetsAt = u.resetsAt ? parseInstant(u.resetsAt) : undefined
  const nextReset =
    resetsAt && !Number.isNaN(resetsAt.getTime()) ? `, next on ${format(resetsAt, 'd MMM')}` : ''
  const notes = [
    u.projected &&
      u.limit &&
      `Projected ${compactQuantity(u.projected)} by ${cycleBound ? 'cycle end' : 'the next reset'}`,
    !cycleBound && `Resets per ${RESET_LABEL[u.reset] ?? u.reset}${nextReset}`,
    limitBehaviorNote(u, freeTrialEnd),
    u.groupBy && `Per ${u.groupBy}, highest shown`,
  ].filter(Boolean)

  return (
    <li className="flex flex-col gap-1">
      {u.limit ? (
        <UsageMeter
          featureName={u.featureName}
          consumed={u.consumed}
          limit={u.limit}
          pct={u.pct}
          elapsedPct={cycleBound ? elapsedPct : undefined}
          overageAllowed={u.overageBehavior === 'allow'}
        />
      ) : (
        <span className="text-xs text-foreground">
          {formatQuantity(u.consumed)} {u.featureName} used, no limit
        </span>
      )}
      {details && notes.length > 0 && (
        <span className="text-xs text-muted-foreground">{notes.join(' · ')}</span>
      )}
    </li>
  )
}
