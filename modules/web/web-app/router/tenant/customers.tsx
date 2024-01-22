import { RouteObject } from 'react-router-dom'

import { Customers } from '@/pages/tenants/customer'

export const customersRoutes: RouteObject = {
  path: 'customers',
  children: [
    {
      index: true,
      element: <Customers />,
    },
  ],
}
