import { disableQuery } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'

import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { activeTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { useEffect } from 'react'

export const useTenant = () => {
  const { tenantSlug } = useTypedParams()
  const queryClient = useQueryClient()

  const { data, isLoading, error } = useQuery(activeTenant, tenantSlug ? undefined : disableQuery)

  useEffect(() => {
    if (tenantSlug && data?.tenant?.slug !== tenantSlug) {
      queryClient.invalidateQueries()
    }
  }, [tenantSlug, data?.tenant?.slug])

  return { tenant: tenantSlug ? data?.tenant : undefined, isLoading, error }
}
