import { RouteObject } from 'react-router-dom'

import { TenantLayoutOutlet } from '@/components/layouts'
import { DashboardPage as Dashboard } from '@/pages/tenants/dashboard'
import { DeveloperSettings } from '@/pages/tenants/developers'
import { TenantSettings } from '@/pages/tenants/settings'
import { billingRoutes } from 'router/tenant/billing'
import { productCatalogRoutes } from 'router/tenant/catalog'
import { customersRoutes } from 'router/tenant/customers'
import { Button } from '@ui2/components'

export const tenantRoutes: RouteObject = {
  path: 'tenant/:tenantSlug',
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

    {
      path: '*',
      element: (
        <div className="items-center justify-center flex flex-col gap-2 w-full">
          <div>Not implemented</div>
          <Button onClick={() => window.history.back()} size="sm">
            Back
          </Button>
        </div>
      ),
    },
  ],
}
