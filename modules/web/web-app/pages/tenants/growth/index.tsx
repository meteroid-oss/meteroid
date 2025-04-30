import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'

export const Growth: FunctionComponent = () => {
  return <Navigate to="goals" />
}

export const GrowthOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout title="Growth">
      <Outlet />
    </TenantPageLayout>
  )
}
