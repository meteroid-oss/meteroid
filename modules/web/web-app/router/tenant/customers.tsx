import { LoaderFunctionArgs, RouteObject } from 'react-router-dom'

import { prefetchQuery } from '@/lib/prefetch'
import { Customer, Customers } from '@/pages/tenants/customer'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'

// Kick off the customer fetch at route-match time rather than waiting for the
// (import-heavy) detail page to mount. The component keeps its own `useQuery`,
// which now resolves against the in-flight request instead of a cold start.
const customerLoader = ({ params }: LoaderFunctionArgs) => {
  const { customerId } = params
  if (customerId) {
    void prefetchQuery(getCustomerById, { id: customerId })
  }
  return null
}

export const customersRoutes: RouteObject = {
  path: 'customers',
  children: [
    {
      index: true,
      element: <Customers />,
      handle: { title: 'Customers' },
    },
    {
      path: ':customerId',
      element: <Customer />,
      handle: { title: 'Customer' },
      loader: customerLoader,
    },
  ],
}
