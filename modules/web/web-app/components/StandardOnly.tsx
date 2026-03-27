import { Navigate, Outlet } from 'react-router-dom'

import { useExpressOrganizationState } from '@/hooks/useIsExpressOrganization'

export const StandardOnly = () => {
  const { isExpress, isLoading } = useExpressOrganizationState()
  if (isLoading) return null
  if (isExpress) return <Navigate to=".." replace />
  return <Outlet />
}
