import { skipToken } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { getSubscriptionUsage } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

const USAGE_STALE_MS = 60_000

export const useSubscriptionUsage = (subscriptionId?: string) =>
  useQuery(getSubscriptionUsage, subscriptionId ? { subscriptionId } : skipToken, {
    staleTime: USAGE_STALE_MS,
  })
