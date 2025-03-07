import { RouteObject } from 'react-router-dom'

import { NotImplemented } from '@/features/NotImplemented'
import { Billing, BillingOutlet } from '@/pages/tenants/billing'
import { Invoice, Invoices } from '@/pages/tenants/invoice'
import { Subscriptions } from '@/pages/tenants/subscription'
import { Subscription } from '@/pages/tenants/subscription/subscription'
import { SubscriptionCreate } from '@/pages/tenants/subscription/subscriptionCreate'

export const billingRoutes: RouteObject = {
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
          path: ':subscriptionId',
          element: <Subscription />,
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
