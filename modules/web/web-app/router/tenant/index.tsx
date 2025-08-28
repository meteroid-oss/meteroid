import { SidebarProvider } from '@ui/components'
import { RouteObject } from 'react-router-dom'

import { TenantLayoutOutlet } from '@/components/layouts'
import { NotImplemented } from '@/features/NotImplemented'
import { EditHubspotIntegrationModal } from '@/features/settings/integrations/EditHubspotIntegrationModal'
import { HubspotIntegrationModal } from '@/features/settings/integrations/HubspotIntegration'
import { PennylaneIntegrationModal } from '@/features/settings/integrations/PennylaneIntegration'
import { StripeIntegrationModal } from '@/features/settings/integrations/StripeIntegration'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
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
    },
    {
      path: 'settings',
      element: <TenantSettings />,
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
      path: 'developers',
      element: <DeveloperSettings />,
    },
    {
      path: 'plan-version/:planVersionId',
      element: <PlanVersionRedirect />,
    },
    productCatalogRoutes,
    customersRoutes,
    billingRoutes,
    growthRoutes,
    {
      path: 'reports',
      element: <ReportsPage />,
    },
    {
      path: '*',
      element: <NotImplemented />,
    },
  ],
}
