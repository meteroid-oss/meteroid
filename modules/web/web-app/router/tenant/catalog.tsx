import { Navigate, RouteObject } from 'react-router-dom'

import { StandardOnly } from '@/components/StandardOnly'
import { NotImplemented } from '@/features/NotImplemented'
import { PlanCreateInitModal } from '@/features/plans/PlanCreateInitModal'
import { DetailsFormModal } from '@/features/plans/create/details/DetailsFormModal'
import { AddonDetails } from '@/pages/tenants/billing/addonDetails'
import { Addons } from '@/pages/tenants/billing/addons'
import { CouponDetails } from '@/pages/tenants/billing/couponDetails'
import { Coupons } from '@/pages/tenants/billing/coupons'
import { CreateAddon } from '@/pages/tenants/billing/createAddon'
import { CreateCoupon } from '@/pages/tenants/billing/createCoupon'
import { EditAddon } from '@/pages/tenants/billing/editAddon'
import { Plans } from '@/pages/tenants/billing/plans'
import { CreateAddOn } from '@/pages/tenants/billing/plans/createAddOn'
import { CreatePriceComponent } from '@/pages/tenants/billing/plans/createPriceComponent'
import { PlanEdit } from '@/pages/tenants/billing/plans/edit'
import { PlanOnboardingComponent } from '@/pages/tenants/billing/plans/onboarding'
import { CatalogOutlet } from '@/pages/tenants/catalog'
import { CreateBillableMetric } from '@/pages/tenants/catalog/createBillableMetric'
import { EditBillableMetric } from '@/pages/tenants/catalog/editBillableMetric'
import { Products } from '@/pages/tenants/catalog/productItems'
import { ProductMetricDetail } from '@/pages/tenants/catalog/productMetricDetail'
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
          path: 'metrics',
          element: <ProductMetrics />,
          handle: { title: 'Metrics' },
          children: [
            {
              element: <StandardOnly />,
              children: [
                {
                  path: 'add-metric',
                  element: <CreateBillableMetric />,
                },
                {
                  path: 'edit/:metricId',
                  element: <EditBillableMetric />,
                },
              ],
            },
          ],
        },
        {
          path: 'metrics/:metricId',
          element: <ProductMetricDetail />,
          handle: { title: 'Metric' },
        },
        {
          path: 'plans',
          children: [
            {
              path: ':planLocalId/:planVersion?',
              element: <PlanEdit />,
              handle: { title: 'Plan' },
              children: [
                {
                  element: <StandardOnly />,
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
                    {
                      path: 'add-addon',
                      element: <CreateAddOn />,
                    },
                  ],
                },
              ],
            },
          ],
        },
        {
          path: 'plans',
          element: <Plans />,
          handle: { title: 'Plans' },
          children: [
            {
              index: true,
            },
            {
              element: <StandardOnly />,
              children: [
                {
                  path: 'add-plan',
                  element: <PlanCreateInitModal />,
                },
              ],
            },
          ],
        },
        {
          element: <StandardOnly />,
          children: [
            {
              path: 'items',
              element: <Products />,
              handle: { title: 'Items' },
            },
            {
              path: 'addons',
              element: <Addons />,
              handle: { title: 'Add-ons' },
              children: [
                {
                  index: true,
                  element: null,
                },
                {
                  path: 'add-addon',
                  element: <CreateAddon />,
                  handle: { title: 'New add-on' },
                },
                {
                  path: ':addonId',
                  element: <AddonDetails />,
                  handle: { title: 'Add-on' },
                },
                {
                  path: 'edit/:addonId',
                  element: <EditAddon />,
                  handle: { title: 'Edit add-on' },
                },
              ],
            },
            {
              path: 'coupons',
              element: <Coupons />,
              handle: { title: 'Coupons' },
              children: [
                {
                  index: true,
                  element: null,
                },
                {
                  path: 'add-coupon',
                  element: <CreateCoupon />,
                  handle: { title: 'New coupon' },
                },
                {
                  path: ':couponLocalId',
                  element: <CouponDetails />,
                  handle: { title: 'Coupon' },
                },
              ],
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
