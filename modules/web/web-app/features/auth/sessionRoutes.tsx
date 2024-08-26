import { jwtDecode } from 'jwt-decode'
import { FC, useMemo } from 'react'
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

const useTokenExpiration = (token: string | undefined) => {
  return useMemo(() => {
    if (!token) return 0
    return (jwtDecode(token).exp ?? 0) * 1000
  }, [token])
}

// prevent access if not authenticated
export const ProtectedRoutes: FC = () => {
  const [session] = useSession()
  const logout = useLogout()
  const meQuery = useQuery(me, undefined, {
    enabled: !!session?.token,
    retry: false,
  })

  const expirationTime = useTokenExpiration(session?.token)

  const [shouldRefresh] = useMemo(() => {
    const now = new Date().getTime()
    return [expirationTime - now < 60 * 60 * 1000]
  }, [expirationTime])

  if (meQuery.isError) {
    return logout('No user profile')
  }

  if (!session?.token) {
    return logout('No session token')
  }

  if (shouldRefresh) {
    logout('Token expiring')
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
