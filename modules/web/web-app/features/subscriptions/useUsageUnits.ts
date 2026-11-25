import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueries } from '@tanstack/react-query'
import { useMemo } from 'react'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { seatCountKey } from '@/features/subscriptions/utils/fees'
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

/**
 * Current seat count per seat ledger (`seatCountKey`: its quantity, else its unit), sharing the
 * cache of the pricing table's seat queries.
 */
export const useSlotCounts = (
  subscriptionId: string,
  ledgers: { unit: string; quantityId?: string }[]
) => {
  const transport = useTransport()
  const keyed = useMemo(() => {
    const byKey = new Map(ledgers.map(ledger => [seatCountKey(ledger), ledger]))
    return [...byKey.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  }, [ledgers])
  const results = useQueries({
    queries: keyed.map(([, ledger]) =>
      createQueryOptions(
        getSlotsValue,
        { subscriptionId, unit: ledger.unit, quantityId: ledger.quantityId || undefined },
        { transport }
      )
    ),
  })
  const counts: Record<string, number> = {}
  keyed.forEach(([key], i) => {
    const value = results[i]?.data?.currentValue
    if (value !== undefined) counts[key] = value
  })
  return counts
}
