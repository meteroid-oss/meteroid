import { RouteObject } from 'react-router-dom'

import { TenantLayoutOutlet } from '@/components/layouts'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
import { TenantSettings } from '@/pages/tenants/settings'
import { Subscriptions } from '@/pages/tenants/subscriptions'
import { billingRoutes } from 'router/tenant/billing'
import { productCatalogRoutes } from 'router/tenant/catalog'
import { customersRoutes } from 'router/tenant/customers'

import { invoiceRoutes } from './invoices'

export const tenantRoutes: RouteObject = {
  path: 'tenant/:tenantSlug',
  element: <TenantLayoutOutlet />,
  children: [
    {
      index: true,
      element: <Dashboard />,
    },
    {
      path: 'subscriptions',
      element: <Subscriptions />,
    },
    {
      path: 'settings',
      element: <TenantSettings />,
    },
    {
      path: 'developers',
      element: <DeveloperSettings />,
    },
    productCatalogRoutes,
    customersRoutes,
    billingRoutes,
    invoiceRoutes,

    {
      path: '*',
      element: <>Not implemented</>,
    },
  ],
}
