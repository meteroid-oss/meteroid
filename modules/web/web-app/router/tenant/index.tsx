import { SidebarProvider } from '@ui/components'
import { RouteObject } from 'react-router-dom'

import { StandardOnly } from '@/components/StandardOnly'
import { TenantLayoutOutlet } from '@/components/layouts'
import { NotImplemented } from '@/features/NotImplemented'
import { MrrReport } from '@/features/reports/charts/MrrReport'
import { RevenueReport } from '@/features/reports/charts/RevenueReport'
import { EditHubspotIntegrationModal } from '@/features/settings/integrations/EditHubspotIntegrationModal'
import { HubspotIntegrationModal } from '@/features/settings/integrations/HubspotIntegration'
import { PennylaneIntegrationModal } from '@/features/settings/integrations/PennylaneIntegration'
import { StripeIntegrationModal } from '@/features/settings/integrations/StripeIntegration'
import { DeadLetterPage } from '@/pages/admin/deadletter'
import { DeadLetterDetailPage } from '@/pages/admin/deadletter-detail'
import { BatchJobPage } from '@/pages/tenants/batchjob'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
import { EventsPage } from '@/pages/tenants/events'
import { HelpPage } from '@/pages/tenants/help'
import { ReportsPage } from '@/pages/tenants/reports'
import { TenantSettings } from '@/pages/tenants/settings'
import { billingRoutes } from 'router/tenant/billing'
import { productCatalogRoutes } from 'router/tenant/catalog'
import { customersRoutes } from 'router/tenant/customers'
import { growthRoutes } from 'router/tenant/growth'
import { PlanVersionRedirect } from 'router/tenant/planVersionRedirect'

export const tenantRoutes: RouteObject = {
  path: ':tenantSlug',
  element: (
    <SidebarProvider>
      <TenantLayoutOutlet />
    </SidebarProvider>
  ),
  children: [
    {
      index: true,
      element: <Dashboard />,
      handle: { title: 'Dashboard' },
    },
    {
      path: 'settings',
      element: <TenantSettings />,
      handle: { title: 'Settings' },
      children: [
        {
          path: 'add-stripe',
          element: <StripeIntegrationModal />,
        },
        {
          path: 'connect-pennylane',
          element: <PennylaneIntegrationModal />,
        },
        {
          path: 'connect-hubspot',
          element: <HubspotIntegrationModal />,
        },
        {
          path: 'edit-hubspot-connection/:connectionId',
          element: <EditHubspotIntegrationModal />,
        },
      ],
    },
    {
      path: 'help',
      element: <HelpPage />,
      handle: { title: 'Help' },
    },
    customersRoutes,
    billingRoutes,
    productCatalogRoutes,
    {
      element: <StandardOnly />,
      children: [
        {
          path: 'reports',
          element: <ReportsPage />,
          handle: { title: 'Reports' },
          children: [
            {
              index: true,
              element: <MrrReport />,
              handle: { title: 'MRR' },
            },
            {
              path: 'revenue',
              element: <RevenueReport />,
              handle: { title: 'Revenue' },
            },
          ],
        },
        {
          path: 'developers',
          handle: { title: 'Developers' },
          children: [
            { index: true, element: <DeveloperSettings /> },
            {
              path: 'batch-jobs/:batchJobId',
              element: <BatchJobPage />,
              handle: { title: 'Batch job' },
            },
          ],
        },
        {
          path: 'events',
          element: <EventsPage />,
          handle: { title: 'Events' },
        },
        {
          path: 'plan-version/:planVersionId',
          element: <PlanVersionRedirect />,
        },
        growthRoutes,
      ],
    },
    {
      path: 'admin',
      children: [
        {
          path: 'dead-letters',
          element: <DeadLetterPage />,
          handle: { title: 'Dead letters' },
        },
        {
          path: 'dead-letters/:deadLetterId',
          element: <DeadLetterDetailPage />,
          handle: { title: 'Dead letter' },
        },
      ],
    },
    {
      path: '*',
      element: <NotImplemented />,
    },
  ],
}
