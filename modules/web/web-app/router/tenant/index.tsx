import { SidebarProvider } from '@ui/components'
import { RouteObject } from 'react-router-dom'

import { StandardOnly } from '@/components/StandardOnly'
import { TenantLayoutOutlet } from '@/components/layouts'
import { editionTenantRoutes } from '@/edition'
import { NotImplemented } from '@/features/NotImplemented'
import { ManageConnectorModal } from '@/features/payments/ManageConnectorModal'
import { PaymentProviderConnectModal } from '@/features/payments/PaymentProviderConnectModal'
import { UpdateCredentialsModal } from '@/features/payments/UpdateCredentialsModal'
import { EditHubspotIntegrationModal } from '@/features/settings/integrations/EditHubspotIntegrationModal'
import { HubspotIntegrationModal } from '@/features/settings/integrations/HubspotIntegration'
import { PennylaneIntegrationModal } from '@/features/settings/integrations/PennylaneIntegration'
import { UserSettings } from '@/pages/profile/userSettings'
import { BatchJobPage } from '@/pages/tenants/batchjob'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { EventsPage } from '@/pages/tenants/events'
import { HelpPage } from '@/pages/tenants/help'
import { SettingsIndexRedirect, TenantSettings } from '@/pages/tenants/settings'
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
      handle: { title: 'Settings', overlay: true },
      children: [
        { index: true, element: <SettingsIndexRedirect /> },
        {
          path: ':tab',
          element: <TenantSettings />,
          children: [
            // Webhooks subpages: the tab reads the URL and renders them in place.
            { path: 'endpoints/:endpointId' },
            { path: 'catalog' },
            {
              path: 'connect-payment-provider/:providerKey',
              element: <PaymentProviderConnectModal />,
            },
            {
              path: 'manage-connector/:connectorId',
              element: <ManageConnectorModal />,
            },
            {
              path: 'manage-connector/:connectorId/credentials',
              element: <UpdateCredentialsModal />,
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
      ],
    },
    {
      path: 'help',
      element: <HelpPage />,
      handle: { title: 'Help' },
    },
    {
      path: 'account',
      element: <UserSettings />,
      handle: { title: 'Account' },
    },
    customersRoutes,
    billingRoutes,
    productCatalogRoutes,
    {
      element: <StandardOnly />,
      children: [
        {
          path: 'developers',
          handle: { title: 'Batch job' },
          children: [
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
    ...editionTenantRoutes(),
    {
      path: '*',
      element: <NotImplemented />,
    },
  ],
}
