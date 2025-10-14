import { TenantPageLayout } from '@/components/layouts'
import { HelpPage as HelpPageTemplate } from '@/features/help/HelpPage'

import type { FunctionComponent } from 'react'

export const HelpPage: FunctionComponent = () => {
  return (
    <TenantPageLayout>
      <HelpPageTemplate />
    </TenantPageLayout>
  )
}
