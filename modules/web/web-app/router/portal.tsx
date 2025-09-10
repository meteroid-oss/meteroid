import { RouteObject } from 'react-router-dom'

import { PortalCheckout } from '@/pages/portal/checkout'
import { PortalCheckoutSuccess } from '@/pages/portal/checkout-success'

export const portalRoutes: RouteObject = {
  children: [
    {
      path: 'checkout',
      children: [
        {
          index: true,
          element: <PortalCheckout />,
        },
        {
          path: 'success',
          element: <PortalCheckoutSuccess />,
        },
      ],
    },
  ],
}
