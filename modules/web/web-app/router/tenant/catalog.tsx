import { Navigate, RouteObject } from 'react-router-dom'

import { Catalog, CatalogOutlet } from '@/pages/tenants/catalog'
import { ProductItems } from '@/pages/tenants/catalog/productItems'
import { ProductMetrics } from '@/pages/tenants/catalog/productMetrics'
import { BillingPeriodModal } from '@/features/billing/plans/details/BillingPeriodModal'
import { Addons } from '@/pages/tenants/billing/addons'
import { Plans } from '@/pages/tenants/billing/plans'
import { CreatePriceComponent } from '@/pages/tenants/billing/plans/createPriceComponent'
import { CreatePriceComponentFee } from '@/pages/tenants/billing/plans/createPriceComponentFee'
import { PlanEdit } from '@/pages/tenants/billing/plans/edit'
import { PlanOnboardingComponent } from '@/pages/tenants/billing/plans/onboarding'

export const productCatalogRoutes: RouteObject = {
  path: 'catalog',
  children: [
    {
      index: true,
      element: <Catalog />,
    },
    {
      path: ':familyExternalId',
      element: <CatalogOutlet />,
      children: [
        {
          index: true,
          element: <Navigate to="items" />,
        },
        {
          path: 'items',
          element: <ProductItems />,
        },
        {
          path: 'metrics',
          element: <ProductMetrics />,
        },
        {
          path: 'plans',
          children: [
            {
              element: <Plans />,
              index: true,
            },
            {
              path: ':planExternalId',
              element: <PlanEdit />,
              children: [
                {
                  path: 'add-component',
                  element: <CreatePriceComponent />,
                },
                {
                  path: 'add-component/:feeType',
                  element: <CreatePriceComponentFee />,
                },
                {
                  path: 'onboarding',
                  element: <PlanOnboardingComponent />,
                },
                {
                  path: 'billing-terms',
                  element: <BillingPeriodModal />,
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
      ],
    },
  ],
}
