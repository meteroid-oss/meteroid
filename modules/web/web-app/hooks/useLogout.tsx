import { Navigate } from 'react-router-dom'

import { useSession } from '@/features/auth'

export function useLogout() {
  const [, , clearSession] = useSession()

  return (message?: string) => {
    if (message) {
      console.error(`${message}, redirecting to the login`)
    }
    clearSession()
    return <Navigate to="/login" />
  }
}
