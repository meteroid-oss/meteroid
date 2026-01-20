import { jwtDecode } from 'jwt-decode'
import { FC, useEffect, useMemo } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { Loader } from '@/features/auth/components/Loader'
import { useSession } from '@/features/auth/session'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

// prevent access if authenticated
export const AnonymousRoutes: FC = () => {
  const [session] = useSession()
  const navigate = useNavigate()

  useEffect(() => {
    if (session?.token) {
      const timeout = setTimeout(() => {
        navigate('/', { replace: true })
      }, 50)
      return () => clearTimeout(timeout)
    }
  }, [session?.token])

  if (session?.token) {
    return <Loader />
  }

  return <Outlet />
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
