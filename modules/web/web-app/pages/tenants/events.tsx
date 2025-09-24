import { TenantPageLayout } from '@/components/layouts'
import { EventsPage as EventsPageComponent } from '@/features/events/EventsPage'

import type { FunctionComponent } from 'react'

export const EventsPage: FunctionComponent = () => {
  return (
    <TenantPageLayout>
      <EventsPageComponent />
    </TenantPageLayout>
  )
}