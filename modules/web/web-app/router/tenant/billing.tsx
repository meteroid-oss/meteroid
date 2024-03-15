import { RouteObject } from 'react-router-dom'

import { NotImplemented } from '@/features/NotImplemented'
import { Billing, BillingOutlet } from '@/pages/tenants/billing'
import { Invoice, Invoices } from '@/pages/tenants/invoice'
import { Subscriptions } from '@/pages/tenants/subscription'
import { SubscriptionCreate } from '@/pages/tenants/subscription/subscriptionCreate'

export const billingRoutes: RouteObject = {
  path: 'billing',
  element: <BillingOutlet />,
  children: [
    {
      index: true,
      element: <Billing />,
    },
    {
      path: 'subscriptions',
      children: [
        {
          index: true,
          element: <Subscriptions />,
        },
        {
          path: 'create',
          element: <SubscriptionCreate />,
        },
      ],
    },
    {
      path: 'invoices',
      children: [
        {
          index: true,
          element: <Invoices />,
        },
        {
          path: ':invoiceId',
          element: <Invoice />,
        },
      ],
    },
    {
      path: '*',
      element: <NotImplemented />,
    },
  ],
}
