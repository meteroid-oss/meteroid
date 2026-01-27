import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSearchParams } from 'react-router-dom'

/**
 * Short aliases for service type names.
 * Usage in URL: ?_sync=stats,subs:20:2
 */
const SERVICE_ALIASES: Record<string, string> = {
  stats: 'meteroid.api.stats.v1.StatsService',
  subs: 'meteroid.api.subscriptions.v1.SubscriptionsService',
  plans: 'meteroid.api.plans.v1.PlansService',
  customers: 'meteroid.api.customers.v1.CustomersService',
  invoices: 'meteroid.api.invoices.v1.InvoicesService',
  metrics: 'meteroid.api.billablemetrics.v1.BillableMetricsService',
  products: 'meteroid.api.products.v1.ProductsService',
  addons: 'meteroid.api.addons.v1.AddOnsService',
  coupons: 'meteroid.api.coupons.v1.CouponsService',
  quotes: 'meteroid.api.quotes.v1.QuotesService',
  tenants: 'meteroid.api.tenants.v1.TenantsService',
  creditnotes: 'meteroid.api.creditnotes.v1.CreditNotesService',
}

const DEFAULT_DURATION = 10 // seconds
const DEFAULT_INTERVAL = 2 // seconds

export type SyncServiceAlias = keyof typeof SERVICE_ALIASES

/**
 * Build a _sync query param string for URL navigation.
 *
 * @example
 * navigate(`/dashboard?${buildSyncParam('stats')}`)
 * navigate(`/dashboard?${buildSyncParam(['stats', 'subs'], { duration: 30 })}`)
 */
export function buildSyncParam(
  services: SyncServiceAlias | SyncServiceAlias[],
  options?: { duration?: number; interval?: number }
): string {
  const serviceList = Array.isArray(services) ? services.join(',') : services
  const parts = [serviceList]

  if (options?.duration || options?.interval) {
    parts.push(String(options.duration ?? DEFAULT_DURATION))
    if (options?.interval) {
      parts.push(String(options.interval))
    }
  }

  return `_sync=${parts.join(':')}`
}

/**
 * Hook that automatically invalidates queries based on URL params.
 *
 * URL format: ?_sync=<services>:<duration>:<interval>
 *
 * Examples:
 *   ?_sync=stats           -> invalidate stats for 10s at 2s intervals
 *   ?_sync=stats:30        -> invalidate stats for 30s at 2s intervals
 *   ?_sync=stats:30:5      -> invalidate stats for 30s at 5s intervals
 *   ?_sync=stats,subs      -> invalidate stats and subs
 *   ?_sync=stats,subs:20:2 -> invalidate both for 20s at 2s intervals
 *
 * Available service aliases: stats, subs, plans, customers, invoices,
 * metrics, products, addons, coupons, quotes, tenants, creditnotes
 */
export function useSyncQueries() {
  const [searchParams, setSearchParams] = useSearchParams()
  const queryClient = useQueryClient()

  const syncParam = searchParams.get('_sync')

  useEffect(() => {
    if (!syncParam) return

    // Parse format: services:duration:interval
    const parts = syncParam.split(':')
    const serviceAliases = parts[0].split(',').map(s => s.trim())
    const duration = (parseInt(parts[1], 10) || DEFAULT_DURATION) * 1000
    const interval = (parseInt(parts[2], 10) || DEFAULT_INTERVAL) * 1000

    // Resolve aliases to full service type names
    const serviceTypeNames = serviceAliases
      .map(alias => SERVICE_ALIASES[alias])
      .filter(Boolean)

    if (serviceTypeNames.length === 0) return

    // Remove _sync param from URL immediately (prevents restart on refresh)
    setSearchParams(prev => {
      prev.delete('_sync')
      return prev
    }, { replace: true })

    const invalidateAll = () => {
      serviceTypeNames.forEach(typeName => {
        queryClient.invalidateQueries({ queryKey: [typeName] })
      })
    }

    // Invalidate immediately, then start periodic invalidation
    invalidateAll()
    const intervalId = setInterval(invalidateAll, interval)

    // Stop after duration expires
    const timeoutId = setTimeout(() => clearInterval(intervalId), duration)

    return () => {
      clearInterval(intervalId)
      clearTimeout(timeoutId)
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
}
