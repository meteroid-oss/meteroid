import { Navigate, RouteObject } from 'react-router-dom'

import { NotImplemented } from '@/features/NotImplemented'
import { DetailsFormModal } from '@/features/billing/plans/create/details/DetailsFormModal'
import { Addons } from '@/pages/tenants/billing/addons'
import { Plans } from '@/pages/tenants/billing/plans'
import { CreatePriceComponent } from '@/pages/tenants/billing/plans/createPriceComponent'
import { PlanEdit } from '@/pages/tenants/billing/plans/edit'
import { PlanOnboardingComponent } from '@/pages/tenants/billing/plans/onboarding'
import { Catalog, CatalogOutlet } from '@/pages/tenants/catalog'
import { CreateBillableMetric } from '@/pages/tenants/catalog/createBillableMetric'
import { Products } from '@/pages/tenants/catalog/productItems'
import { ProductMetrics } from '@/pages/tenants/catalog/productMetrics'

export const productCatalogRoutes: RouteObject = {
  path: 'catalog',
  children: [
    {
      index: true,
      element: <Catalog />,
    },
    {
      path: ':familyLocalId',
      element: <CatalogOutlet />,
      children: [
        {
          index: true,
          element: <Navigate to="items" />,
        },
        {
          path: 'items',
          element: <Products />,
        },
        {
          path: 'metrics',
          element: <ProductMetrics />,
          children: [
            {
              path: 'add-metric',
              element: <CreateBillableMetric />,
            },
          ],
        },
        {
          path: 'plans',
          children: [
            {
              element: <Plans />,
              index: true,
            },
            {
              path: ':planLocalId/:planVersion?',
              element: <PlanEdit />,
              children: [
                {
                  path: 'add-component',
                  element: <CreatePriceComponent />,
                },

                {
                  path: 'onboarding',
                  element: <PlanOnboardingComponent />,
                },
                {
                  path: 'add-metric',
                  element: <CreateBillableMetric />,
                },
                {
                  path: 'edit-overview',
                  element: <DetailsFormModal />,
                },

                // TODO component/:priceComponentId
              ],
            },
          ],
        },
        {
          path: 'addons',
          element: <Addons />,
        },
        {
          path: '*',
          element: <NotImplemented />,
        },
      ],
    },
  ],
}
