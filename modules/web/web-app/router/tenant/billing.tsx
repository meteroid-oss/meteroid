import { Navigate, RouteObject } from 'react-router-dom'

import { Billing, BillingOutlet } from '@/pages/tenants/billing'
import { Invoice, Invoices } from '@/pages/tenants/invoice'
import { Subscriptions } from '@/pages/tenants/subscriptions'
import { NotImplemented } from '@/features/NotImplemented'

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
      element: <Subscriptions />,
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
