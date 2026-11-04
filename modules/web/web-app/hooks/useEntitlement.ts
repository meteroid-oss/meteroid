import { callUnaryMethod, createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQuery } from '@tanstack/react-query'

import { useTypedParams } from '@/lib/utils/params'
import { getFeatures } from '@/rpc/api/features/v1/features-FeaturesService_connectquery'

/**
 * The gate codes core reads, or that more than one module reads. A module keeps the codes only it
 * reads in its own `gates.ts`.
 */
export const entitlementCodes = {
  LICENSING: 'licensing',
  RBAC: 'rbac',
  ADMIN_SANDBOXES: 'admin_sandboxes',
  SSO: 'sso',
  WHITELABEL: 'whitelabel',
} as const

/**
 * The features of the organization and tenant in the URL. The request names neither (the header
 * does), so they are part of the key: another tenant's answer is never served from the cache.
 */
export const useFeaturesQuery = () => {
  const transport = useTransport()
  const { organizationSlug, tenantSlug } = useTypedParams()
  const { queryKey } = createQueryOptions(getFeatures, {}, { transport })
  return useQuery({
    queryKey: [...queryKey, { organizationSlug, tenantSlug }],
    queryFn: ({ signal }) => callUnaryMethod(transport, getFeatures, {}, { signal }),
  })
}

/**
 * Resolve a single entitlement by code.
 *
 * UX-only: gates the display/affordance. Server-side checks remain authoritative.
 * When the code is absent, returns `options.default` (default `false`).
 */
export const useEntitlement = (code: string, options?: { default?: boolean }): boolean => {
  const { data } = useFeaturesQuery()
  const entry = data?.entitlements?.find(e => e.code === code)
  return entry === undefined ? (options?.default ?? false) : entry.enabled
}
