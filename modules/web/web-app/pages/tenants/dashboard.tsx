import { TenantPageLayout } from '@/components/layouts'
import { Dashboard } from '@/features/dashboard/Dashboard'

import type { FunctionComponent } from 'react'

export const DashboardPage: FunctionComponent = () => {
  return (
    <TenantPageLayout title="Home">
      <Dashboard />
    </TenantPageLayout>
  )
}
