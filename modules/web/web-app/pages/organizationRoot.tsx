import { Navigate } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { useQuery } from '@/lib/connectrpc'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const OrganizationRoot: React.FC = () => {
  const tenantsQuery = useQuery(listTenants)

  const tenants = (tenantsQuery.data?.tenants ?? []).sort((a, b) => b.environment - a.environment)

  if (tenantsQuery.isLoading || tenantsQuery.isPending) {
    return <Loading />
  }

  // TODO last accessed organization/tenant
  if (tenants.length >= 1) {
    return <Navigate to={tenants[0].slug} />
  }

  return <Navigate to="tenants/new" />
}
