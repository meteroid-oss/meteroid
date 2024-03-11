import { jwtDecode } from 'jwt-decode'
import { FC } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { useSession } from '@/features/auth'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

// prevent access if authenticated
export const AnonymousRoutes: FC = () => {
  const [session] = useSession()

  if (session?.token) return <Navigate to="/" replace />

  return <Outlet /> // TODO this requires an error, any other solution ?
}

// prevent access if not authenticated
export const ProtectedRoutes: FC = () => {
  const meQuery = useQuery(me)
  const logout = useLogout()
  const [session] = useSession()

  if (meQuery.isError) {
    return logout('No user profile')
  }

  if (!session?.token) {
    return logout('No session token')
  }

  const expiringMs = (jwtDecode(session.token).exp ?? 0) * 1000
  if (new Date().getTime() > expiringMs) {
    return logout('Session expired')
  }

  if (meQuery.isLoading) {
    return <Loading />
  }

  return <Outlet />
}
export const protectedRoute = (Layout: FC<{ children: JSX.Element }>): React.ReactNode => {
  return (
    <Layout>
      <ProtectedRoutes />
    </Layout>
  )
}
