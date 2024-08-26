import { RouteObject } from 'react-router-dom'

import { TenantLayoutOutlet } from '@/components/layouts'
import { NotImplemented } from '@/features/NotImplemented'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
import { ReportsPage } from '@/pages/tenants/reports'
import { TenantSettings } from '@/pages/tenants/settings'
import { billingRoutes } from 'router/tenant/billing'
import { productCatalogRoutes } from 'router/tenant/catalog'
import { customersRoutes } from 'router/tenant/customers'
import { growthRoutes } from 'router/tenant/growth'

export const tenantRoutes: RouteObject = {
  path: ':tenantSlug',
  element: <TenantLayoutOutlet />,
  children: [
    {
      index: true,
      element: <Dashboard />,
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
