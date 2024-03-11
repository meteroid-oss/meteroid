import { RouteObject } from 'react-router-dom'

import { Invoice, Invoices } from '@/pages/tenants/invoice'

export const billingRoutes: RouteObject = {
  path: 'billing',
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
}
