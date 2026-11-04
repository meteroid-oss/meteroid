import { Navigate, useNavigate } from 'react-router-dom'

import { useSession } from '@/features/auth'
import { clearAllPortalTokens } from '@/features/auth/portalToken'
import { queryClient } from '@/lib/react-query'

export function useLogout() {
  const [, , clearSession] = useSession()
  const navigate = useNavigate()

  return (message?: string) => {
    if (message) {
      console.error(`${message}, redirecting to the login`)
    }
    queryClient.clear()
    clearSession()
    // Also drop any persisted customer-portal token on logout.
    clearAllPortalTokens()
    navigate('/login')
    return <Navigate to="/login" />
  }
}
