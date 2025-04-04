import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import SidebarMenu from '@/components/SidebarMenu'
import { TenantPageLayout } from '@/components/layouts'

export const Growth: FunctionComponent = () => {
  return <Navigate to="goals" />
}

export const GrowthOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout
      title="Growth"
      innerMenu={
        <SidebarMenu
          items={[
            {
              label: 'Goals',
              to: 'goals',
            },
            {
              // similar to cost control, so thresholds into actions
              // cost control can be linked to cost metrics, but it's also
              label: 'Opportunities',
              to: 'opportunities',
            },
            {
              label: 'Experiments',
              to: 'experiments',
            },
            {
              label: 'Churn Management',
              to: 'churn',
            },
            {
              // cf ignition
              label: 'Launchpad',
              to: 'launchpad',
            },
          ]}
        />
      }
    >
      <Outlet />
    </TenantPageLayout>
  )
}
