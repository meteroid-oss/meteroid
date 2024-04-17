import { Navigate } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

/**
 * This checks the onboarding status of the user, and redirect accordingly
 */
export const Root: React.FC = () => {
  // in the updated version :
  // - we create User, Organization and Tenant on the first login
  // therefore no initUser or anything is done on this page.
  // we simply load : user, with the single org and all tenants
  // we load the tenants for the user and redirect to the tenant page

  const logout = useLogout()

  const meQuery = useQuery(me)

  const tenantsQuery = useQuery(listTenants)

  if (tenantsQuery.isError) {
    return logout('Tenants error')
  }

  if (meQuery.isLoading || tenantsQuery.isLoading) {
    return <Loading />
  }

  // TODO localstorage for last accessed tenant
  if (tenantsQuery.data?.tenants?.length) {
    return <Navigate to={`/tenant/${tenantsQuery.data.tenants[0].slug}`} />
  }

  return <Navigate to="/tenants/new" />
}
