import { Navigate } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { useUserPreferences } from '@/features/updates/useUserPreferences'
import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const OrganizationRoot: React.FC = () => {
  const { organizationSlug } = useTypedParams()
  const tenantsQuery = useQuery(listTenants)
  const { preferences, isLoading: preferencesLoading } = useUserPreferences()

  const tenants = (tenantsQuery.data?.tenants ?? []).sort((a, b) => b.environment - a.environment)

  if (tenantsQuery.isLoading || tenantsQuery.isPending || preferencesLoading) {
    return <Loading />
  }

  if (tenants.length >= 1) {
    const lastSlug = organizationSlug ? preferences.lastTenantByOrg?.[organizationSlug] : undefined
    const lastVisited = tenants.find(t => t.slug === lastSlug)
    const target = lastVisited ?? tenants[0]
    return <Navigate to={target.slug} />
  }

  return <Navigate to="tenants/new" />
}
