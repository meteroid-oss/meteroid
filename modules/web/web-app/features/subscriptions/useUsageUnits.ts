import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueries } from '@tanstack/react-query'
import { useMemo } from 'react'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { getSlotsValue } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { MetricUnit } from '@/features/billing/metrics/units'

const METRIC_STALE_MS = 5 * 60_000

/** The unit of each metric, keyed by id; a metric is missing until it loads (or if it fails). */
export const useUsageUnits = (metricIds: string[]) => {
  const transport = useTransport()
  const ids = useMemo(() => [...new Set(metricIds.filter(Boolean))].sort(), [metricIds])
  const results = useQueries({
    queries: ids.map(id => ({
      ...createQueryOptions(getBillableMetric, { id }, { transport }),
      staleTime: METRIC_STALE_MS,
    })),
  })
  const units: Record<string, MetricUnit | undefined> = {}
  ids.forEach((id, i) => {
    const metric = results[i]?.data?.billableMetric
    if (metric) units[id] = metricUnitFromProto(metric)
  })
  return { units, isLoading: results.some(r => r.isLoading) }
}

/** Current slot count per slot unit, sharing the cache of the pricing table's slot queries. */
export const useSlotCounts = (subscriptionId: string, slotUnits: string[]) => {
  const transport = useTransport()
  const units = useMemo(() => [...new Set(slotUnits)].sort(), [slotUnits])
  const results = useQueries({
    queries: units.map(unit =>
      createQueryOptions(getSlotsValue, { subscriptionId, unit }, { transport })
    ),
  })
  const counts: Record<string, number> = {}
  units.forEach((unit, i) => {
    const value = results[i]?.data?.currentValue
    if (value !== undefined) counts[unit] = value
  })
  return counts
}
