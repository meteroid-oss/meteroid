import { skipToken } from '@connectrpc/connect-query'

import { type CheckAmounts, type Target, isTargetReady, targetRequest } from '@/features/entitlements/explain/checkRequest'
import { useQuery } from '@/lib/connectrpc'
import { checkEntitlement } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

const explainBody = (feature: string, target: Target, amounts: CheckAmounts) => ({
  feature,
  explain: true,
  ...targetRequest(target),
  requested: amounts.requested,
  currentCount: amounts.currentCount,
})

/** The check with `explain` for one feature, evaluated now; idle until `enabled` and the target is complete. */
export const useExplain = (
  customerId: string,
  featureCode: string | undefined,
  target: Target,
  enabled = true,
  amounts: CheckAmounts = {}
) =>
  useQuery(
    checkEntitlement,
    enabled && customerId && featureCode && isTargetReady(target)
      ? { customerId, ...explainBody(featureCode, target, amounts) }
      : skipToken
  )

export const subscriptionName = (s: Pick<Subscription, 'planName' | 'version'>) => `${s.planName} v${s.version}`
