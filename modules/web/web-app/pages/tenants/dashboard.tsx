import { TenantPageLayout } from '@/components/layouts'
import { NotImplementedPageEmptyState } from '@/features/temp/NotImplementedPage'

import type { FunctionComponent } from 'react'

export const Dashboard: FunctionComponent = () => {
  return (
    <TenantPageLayout title="Home">
      <NotImplementedPageEmptyState />
    </TenantPageLayout>
  )
}
