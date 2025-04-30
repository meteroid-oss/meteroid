import { FunctionComponent } from 'react'
import { Outlet } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'

export const CatalogOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout title="Offering">
      <Outlet />
    </TenantPageLayout>
  )
}
