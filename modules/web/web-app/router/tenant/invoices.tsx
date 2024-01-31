import { RouteObject } from 'react-router-dom'

import { Invoice, Invoices } from '@/pages/tenants/invoice'

export const invoiceRoutes: RouteObject = {
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
}
