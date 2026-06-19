import { Outlet, useParams } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { NoOrganizationAccess } from '@/features/auth/NoOrganizationAccess'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const OrganizationGuard: React.FC = () => {
  const { organizationSlug } = useParams()
  const meQuery = useQuery(me)

  if (meQuery.isLoading || meQuery.isPending) {
    return <Loading />
  }

  const organizations = meQuery.data?.organizations ?? []
  const hasAccess = organizations.some(org => org.slug === organizationSlug)

  if (!hasAccess) {
    return <NoOrganizationAccess organizationSlug={organizationSlug} />
  }

  return <Outlet />
}
