import { TenantPageLayout } from '@/components/layouts'
import { DeveloperSettings as DeveloperSettingsTemplate } from '@/features/settings/DeveloperSettings'

import type { FunctionComponent } from 'react'

export const DeveloperSettings: FunctionComponent = () => {
  return (
    <TenantPageLayout>
      <DeveloperSettingsTemplate />
    </TenantPageLayout>
  )
}
