
import { TenantPageLayout } from '@/components/layouts'
import { TenantSettings as TenantSettingsTemplate } from '@/features/settings/TenantSettings'

import type { FunctionComponent } from 'react'
import { Outlet } from 'react-router-dom'

export const TenantSettings: FunctionComponent = () => {
  return (
    <> <TenantPageLayout title="Settings" displayTitle>
      <TenantSettingsTemplate />
      <Outlet />

    </TenantPageLayout>
    </>
  )
}
