import { Navigate } from 'react-router-dom'

import { useSession } from '@/features/auth'

export function useLogout() {
  const [, setSession] = useSession()

  return (message?: string) => {
    if (message) {
      console.error(`${message}, redirecting to the login`)
    }
    setSession(null)
    return <Navigate to="/login" />
  }
}
