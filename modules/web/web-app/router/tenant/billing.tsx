import { Navigate, RouteObject } from 'react-router-dom'

import { BillingPeriodModal } from '@/features/billing/plans/details/BillingPeriodModal'
import { Billing, BillingOutlet } from '@/pages/tenants/billing'
import { Addons } from '@/pages/tenants/billing/addons'
import { Plans } from '@/pages/tenants/billing/plans'
import { CreatePriceComponent } from '@/pages/tenants/billing/plans/createPriceComponent'
import { CreatePriceComponentFee } from '@/pages/tenants/billing/plans/createPriceComponentFee'
import { PlanEdit } from '@/pages/tenants/billing/plans/edit'
import { PlanOnboardingComponent } from '@/pages/tenants/billing/plans/onboarding'

export const billingRoutes: RouteObject = {
  path: 'billing',
  children: [
    {
      index: true,
      element: <Billing />,
    },
    {
      path: ':familyExternalId',
      element: <BillingOutlet />,
      children: [
        {
          index: true,
          element: <Navigate to="plans" />,
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
