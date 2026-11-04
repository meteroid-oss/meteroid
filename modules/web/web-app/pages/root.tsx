import { Button } from '@ui/components'
import { Navigate } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { INVITE_TOKEN_KEY } from '@/features/auth/invite'
import { useUserPreferences } from '@/features/updates/useUserPreferences'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

/**
 * This checks the onboarding status of the user, and redirect accordingly
 */
export const Root: React.FC = () => {
  const logout = useLogout()

  const meQuery = useQuery(me)
  const getInstanceQuery = useQuery(getInstance)
  const { preferences } = useUserPreferences()

  const organizations = (meQuery.data?.organizations ?? []).sort(
    (a, b) => Number(a.createdAt) - Number(b.createdAt)
  )

  // Post-auth redirects race to '/' (e.g. AnonymousRoutes vs LoginForm), so an
  // invite stashed before sign-in is resumed here instead of being dropped.
  if (sessionStorage.getItem(INVITE_TOKEN_KEY)) {
    return <Navigate to="/invite-authenticated" replace />
  }

  if (meQuery.isError) {
    return logout('Failed to load user organizations (/me)')
  }

  if (meQuery.isLoading || getInstanceQuery.isLoading) {
    return <Loading />
  }

  if (organizations.length >= 1) {
    const lastVisited = organizations.find(o => o.slug === preferences.lastOrganizationSlug)
    const target = lastVisited ?? organizations[0]
    return <Navigate to={`/${target.slug}`} />
  }

  if (getInstanceQuery.data?.instanceInitiated && !getInstanceQuery.data.multiOrganizationEnabled) {
    return (
      <div className="p-10">
        <div>
          You don&apos;t have access to this instance. Request an invite link to your admin.
        </div>
        <div>
          <Button onClick={() => logout()}>Logout</Button>
        </div>
      </div>
    )
  }

  if (!meQuery.data?.user?.onboarded) {
    return <Navigate to="/onboarding/user" />
  }

  return <Navigate to="/onboarding/organization" />
}
