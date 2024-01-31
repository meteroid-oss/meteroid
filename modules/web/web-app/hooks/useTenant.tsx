import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { activeTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const useTenant = () => {
  const { tenantSlug } = useTypedParams()

  const { data, isLoading, error } = useQuery(activeTenant, tenantSlug ? undefined : disableQuery)

  return { tenant: tenantSlug ? data?.tenant : undefined, isLoading, error }
}
