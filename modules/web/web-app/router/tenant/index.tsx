import { RouteObject } from 'react-router-dom'

import { TenantLayoutOutlet } from '@/components/layouts'
import { Customers } from '@/pages/tenants/customers'
import { Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
import { TenantSettings } from '@/pages/tenants/settings'
import { Subscriptions } from '@/pages/tenants/subscriptions'
import { billingRoutes } from 'router/tenant/billing'
import { productCatalogRoutes } from 'router/tenant/catalog'

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
      path: 'customers',
      element: <Customers />,
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
    billingRoutes,
    invoiceRoutes,

    {
      path: '*',
      element: <>Not implemented</>,
    },
  ],
}
