import { Badge } from '@md/ui'
import { format } from 'date-fns'

import { productSections } from '@/features/entitlements/productGroups'
import { usePathLabel } from '@/features/resources/hooks'
import { UsageMeter } from '@/features/subscriptions/UsageMeter'
import { compactQuantity, formatQuantity, parseInstant } from '@/lib/utils/format'

import type { SubscriptionGrant } from '@/rpc/api/entitlements/v1/models_pb'
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
    return u.gracePct ? `Deny at limit + ${u.gracePct}% grace` : 'Deny at limit'
  }
  if (u.overageBehavior !== 'allow') return undefined
  return freeTrialEnd
    ? `Overage allowed · trial usage not billed until ${format(freeTrialEnd, 'd MMM')}`
    : 'Overage allowed'
}

/**
 * When another subscription decides the limit shown: what this one grants, and which decides,
 * e.g. "Grants 1k; Pro on the customer decides 10k".
 */
export const ownGrantNote = (
  grant: SubscriptionGrant | undefined,
  limit?: string,
  pathLabel: (path: string) => string = path => path
) => {
  if (!grant) return undefined
  const value = grant.value?.value
  const own =
    value?.case !== 'meteredValue'
      ? 'nothing of its own'
      : !value.value.enabled
        ? 'nothing'
        : value.value.limit
          ? compactQuantity(value.value.limit)
          : 'unlimited'
  const where = grant.decidingResourcePath ? `on ${pathLabel(grant.decidingResourcePath)}` : 'on the customer'
  const decides = limit ? `decides ${compactQuantity(limit)}` : 'decides'
  return `Grants ${own}; ${grant.decidingPlanName ?? 'another subscription'} ${where} ${decides}`
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
  customerId,
  groupByProduct = true,
}: {
  usage: SubscriptionFeatureUsage[]
  /** Names the resource of another subscription deciding a limit. */
  customerId?: string
  /** The cycle's elapsed share, marked on cycle-bound meters. */
  elapsedPct?: number
  /** Projection, reset rule and hard limit under each meter. */
  details?: boolean
  /** Set while the subscription is in a free trial: when it ends. */
  freeTrialEnd?: Date
  /** Off for a summary ranked across products, e.g. the most used meters. */
  groupByProduct?: boolean
}) => {
  const pathLabel = usePathLabel(customerId)
  const list = (items: SubscriptionFeatureUsage[]) => (
    <ul className="flex flex-col gap-3">
      {items.map(u => (
        <FeatureUsageItem
          key={u.featureId}
          usage={u}
          elapsedPct={elapsedPct}
          details={details}
          freeTrialEnd={freeTrialEnd}
          pathLabel={pathLabel}
        />
      ))}
    </ul>
  )
  const sections = groupByProduct ? productSections(usage, u => u.product) : undefined
  if (!sections) return list(usage)
  return (
    <div className="flex flex-col gap-4">
      {sections.map(section => (
        <section key={section.key} className="flex flex-col gap-2">
          <span className="text-xs font-medium text-muted-foreground">{section.label}</span>
          {list(section.items)}
        </section>
      ))}
    </div>
  )
}

const FeatureUsageItem = ({
  usage: u,
  elapsedPct,
  details,
  freeTrialEnd,
  pathLabel,
}: {
  usage: SubscriptionFeatureUsage
  elapsedPct?: number
  details: boolean
  freeTrialEnd?: Date
  pathLabel: (path: string) => string
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
  const grant = ownGrantNote(u.ownGrant, u.limit, pathLabel)

  return (
    <li className="flex flex-col gap-1">
      {u.consumed === undefined ? (
        <span className="text-xs text-foreground">
          {u.featureName} <span className="text-muted-foreground">· Usage unavailable</span>
        </span>
      ) : u.limit ? (
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
      {(u.refused || (details && notes.length > 0)) && (
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          {u.refused && (
            <Badge variant="destructive" size="sm">
              Limit reached
            </Badge>
          )}
          {details && notes.join(' · ')}
        </span>
      )}
      {grant && <span className="text-xs text-muted-foreground">{grant}</span>}
    </li>
  )
}
