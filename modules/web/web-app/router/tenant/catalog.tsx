import { Navigate, RouteObject } from 'react-router'

import { StandardOnly } from '@/components/StandardOnly'
import { NotFound } from '@/features/NotFound'
import { PlanCreateInitModal } from '@/features/plans/PlanCreateInitModal'
import { DetailsFormModal } from '@/features/plans/create/details/DetailsFormModal'
import { useBasePath } from '@/hooks/useBasePath'
import { Addons } from '@/pages/tenants/billing/addons'
import { Coupons } from '@/pages/tenants/billing/coupons'
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
import { FeatureCreate, FeatureDetail, FeatureEdit, Features } from '@/pages/tenants/catalog/features'
import { Products } from '@/pages/tenants/catalog/productItems'
import { ProductMetricDetail } from '@/pages/tenants/catalog/productMetricDetail'
import { ProductMetrics } from '@/pages/tenants/catalog/productMetrics'

const CatalogRedirect = ({ to }: { to: string }) => {
  const basePath = useBasePath()
  return <Navigate to={`${basePath}/${to}`} replace />
}

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
        // Seat rules live on the seat product, rate cards on their metric.
        {
          path: 'quantities',
          element: <CatalogRedirect to="items" />,
        },
        {
          path: 'rate-cards',
          element: <CatalogRedirect to="metrics" />,
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
                      // Offered from the plan's "Add-ons" section: older links open it.
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
              path: 'features',
              element: <Features />,
              handle: { title: 'Features' },
              children: [
                { index: true, element: null },
                { path: 'create', element: <FeatureCreate />, handle: { title: 'New feature' } },
                { path: ':featureId', element: <FeatureDetail /> },
                { path: 'edit/:featureId', element: <FeatureEdit />, handle: { title: 'Edit feature' } },
              ],
            },
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
                  // "New add-on" and the add-on open beside the list, shown by the page.
                  path: 'add-addon',
                  element: null,
                  handle: { title: 'New add-on' },
                },
                {
                  path: ':addonId',
                  element: null,
                  handle: { title: 'Add-on' },
                },
                {
                  // Edited in its panel now: older links open it there.
                  path: 'edit/:addonId',
                  element: <EditAddon />,
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
                  // Shown beside the list by the page: the row stays in view.
                  path: ':couponLocalId',
                  element: null,
                  handle: { title: 'Coupon' },
                },
              ],
            },
          ],
        },
        {
          path: '*',
          element: <NotFound />,
        },
      ],
    },
  ],
}
