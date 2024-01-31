import { useLogout } from '@/hooks/useLogout'

import type { FunctionComponent } from 'react'

export const Logout: FunctionComponent = () => {
  const logout = useLogout()

  return logout()
}
