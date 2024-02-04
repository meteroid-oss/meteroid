import { RouteObject } from 'react-router-dom'

import { Customer, Customers } from '@/pages/tenants/customer'

export const customersRoutes: RouteObject = {
  path: 'customers',
  children: [
    {
      index: true,
      element: <Customers />,
    },
    {
      path: ':customerId',
      element: <Customer />,
    },
  ],
}
