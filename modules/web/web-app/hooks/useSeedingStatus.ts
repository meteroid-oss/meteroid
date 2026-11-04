import { skipToken } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getTenantSeedingStatus } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

// Services whose data grows progressively during sandbox seeding, so the
// dashboard charts (revenue, MRR, signups…) reflect the incoming data live.
const LIVE_SEED_SERVICES = [
  'meteroid.api.stats.v1.StatsService',
  'meteroid.api.analytics.v1.AnalyticsService',
  'meteroid.api.customers.v1.CustomersService',
  'meteroid.api.subscriptions.v1.SubscriptionsService',
  'meteroid.api.invoices.v1.InvoicesService',
]

const REFRESH_INTERVAL_MS = 5000

/**
 * Polls a seeded sandbox's data-generation status. While generation is in
 * progress it refetches every few seconds and periodically invalidates the
 * stats/customers/subscriptions/invoices queries so the dashboard charts
 * refresh live as data lands; when it flips to complete it invalidates all
 * queries once so everything else settles without a manual refresh.
 */
export function useSeedingStatus(tenantId: string | undefined) {
  const queryClient = useQueryClient()
  const wasInProgress = useRef(false)

  const query = useQuery(getTenantSeedingStatus, tenantId ? { tenantId } : skipToken, {
    refetchInterval: data => (data?.state?.data?.inProgress ? 3000 : false),
  })

  const inProgress = query.data?.inProgress ?? false

  useEffect(() => {
    if (inProgress) {
      wasInProgress.current = true
    } else if (wasInProgress.current) {
      wasInProgress.current = false
      queryClient.invalidateQueries()
    }
  }, [inProgress, queryClient])

  // Live-refresh the dashboard charts while seeding is still running.
  useEffect(() => {
    if (!inProgress) return

    const refresh = () => {
      LIVE_SEED_SERVICES.forEach(serviceName => {
        queryClient.invalidateQueries({ queryKey: ['connect-query', { serviceName }] })
      })
    }

    const intervalId = setInterval(refresh, REFRESH_INTERVAL_MS)
    return () => clearInterval(intervalId)
  }, [inProgress, queryClient])

  return {
    inProgress,
    pending: query.data?.pendingSubscriptions ?? 0,
    failed: query.data?.failed ?? false,
    failureReason: query.data?.failureReason,
  }
}
