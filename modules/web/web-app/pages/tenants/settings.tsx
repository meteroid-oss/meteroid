import { Outlet } from 'react-router'

import { TenantPageLayout } from '@/components/layouts'
import { TenantSettings as TenantSettingsTemplate } from '@/features/settings/TenantSettings'

import type { FunctionComponent } from 'react'

export const TenantSettings: FunctionComponent = () => {
  return (
    <TenantPageLayout>
      <TenantSettingsTemplate />
      <Outlet />
    </TenantPageLayout>
  )
}
