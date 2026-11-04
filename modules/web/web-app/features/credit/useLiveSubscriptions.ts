import { useQuery } from '@/lib/connectrpc'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

/** Subscriptions credit can still be given to: started, trialing or about to start. */
export const LIVE_STATUSES = [
  SubscriptionStatus.PENDING,
  SubscriptionStatus.TRIALING,
  SubscriptionStatus.ACTIVE,
]

export const useLiveSubscriptions = (customerId: string, enabled = true) => {
  const query = useQuery(
    listSubscriptions,
    { customerId, status: LIVE_STATUSES, pagination: { perPage: 50, page: 0 } },
    { enabled }
  )
  return query.data?.subscriptions ?? []
}
