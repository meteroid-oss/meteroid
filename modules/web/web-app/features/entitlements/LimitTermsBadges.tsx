import { Badge, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'

import { parseThresholds } from '@/features/thresholds/thresholds'

import {
  limitAlertsLabel,
  limitBehaviorHelp,
  limitBehaviorLabel,
  limitGroupLabel,
  limitTermsOf,
} from './utils'

import type { PendingEntitlementSpec } from './creation/types'
import type { LimitTerms } from './utils'
import type { EffectiveEntitlement, ResolvedEntitlement } from '@/rpc/api/entitlements/v1/models_pb'

export const termsOfValue = (
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): LimitTerms | undefined => (value.case === 'metered' ? limitTermsOf(value.value) : undefined)

export const termsOfSpec = (spec: PendingEntitlementSpec): LimitTerms | undefined => {
  if (spec.featureType !== 'metered') return undefined
  const grace = Number(spec.gracePct?.trim())
  return {
    limit: spec.limit?.trim() || undefined,
    groupBy: spec.groupBy?.trim() || undefined,
    overage: spec.overageType === 'allow' ? 'allow' : 'block',
    gracePct: Number.isInteger(grace) && grace > 0 ? grace : undefined,
    thresholds: specThresholds(spec.thresholds),
  }
}

const specThresholds = (text: string | undefined): number[] => {
  try {
    return parseThresholds(text ?? '')
  } catch {
    return []
  }
}

/** What a metered limit does once reached, and the property it applies per; nothing otherwise. */
export const LimitTermsBadges = ({ terms }: { terms?: LimitTerms }) => {
  if (!terms) return null
  const behavior = limitBehaviorLabel(terms)
  const group = limitGroupLabel(terms)
  const alerts = limitAlertsLabel(terms)
  return (
    <>
      {behavior && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Badge size="sm" variant="outline">
              {behavior}
            </Badge>
          </TooltipTrigger>
          <TooltipContent className="max-w-56">{limitBehaviorHelp(terms)}</TooltipContent>
        </Tooltip>
      )}
      {group && (
        <Badge size="sm" variant="outline">
          {group}
        </Badge>
      )}
      {alerts && (
        <Badge size="sm" variant="outline">
          {alerts}
        </Badge>
      )}
    </>
  )
}
