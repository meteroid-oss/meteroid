import { Navigate, useNavigate } from 'react-router-dom'

import { useSession } from '@/features/auth'
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
    navigate('/login')
    return <Navigate to="/login" />
  }
}
