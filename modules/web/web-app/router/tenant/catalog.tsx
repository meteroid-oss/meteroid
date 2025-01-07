import { Navigate, RouteObject } from 'react-router-dom'

import { NotImplemented } from '@/features/NotImplemented'
import { PlanCreateInitModal } from '@/features/plans/PlanCreateInitModal'
import { DetailsFormModal } from '@/features/plans/create/details/DetailsFormModal'
import { Addons } from '@/pages/tenants/billing/addons'
import { CouponDetails } from '@/pages/tenants/billing/couponDetails'
import { Coupons } from '@/pages/tenants/billing/coupons'
import { CreateAddon } from '@/pages/tenants/billing/createAddon'
import { CreateCoupon } from '@/pages/tenants/billing/createCoupon'
import { Plans } from '@/pages/tenants/billing/plans'
import { CreatePriceComponent } from '@/pages/tenants/billing/plans/createPriceComponent'
import { PlanEdit } from '@/pages/tenants/billing/plans/edit'
import { PlanOnboardingComponent } from '@/pages/tenants/billing/plans/onboarding'
import { CatalogOutlet } from '@/pages/tenants/catalog'
import { CreateBillableMetric } from '@/pages/tenants/catalog/createBillableMetric'
import { Products } from '@/pages/tenants/catalog/productItems'
import { ProductMetrics } from '@/pages/tenants/catalog/productMetrics'

export const productCatalogRoutes: RouteObject = {
  children: [
    {
      element: <CatalogOutlet />,
      children: [
        {
          index: true,
          element: <Navigate to="plans" />,
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
          path: 'plans',
          element: <Plans />,
          children: [
            {
              index: true,
            },
            {
              path: 'add-plan',
              element: <PlanCreateInitModal />,
            },
          ],
        },
        {
          path: 'addons',
          element: <Addons />,
          children: [
            {
              index: true,
              element: null,
            },
            {
              path: 'add-addon',
              element: <CreateAddon />,
            },
          ],
        },
        {
          path: 'coupons',
          element: <Coupons />,
          children: [
            {
              index: true,
              element: null,
            },
            {
              path: 'add-coupon',
              element: <CreateCoupon />,
            },
            {
              path: ':couponLocalId',
              element: <CouponDetails />,
            },
          ],
        },
        {
          path: '*',
          element: <NotImplemented />,
        },
      ],
    },
  ],
}
