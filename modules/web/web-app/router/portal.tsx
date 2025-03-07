import { RouteObject } from 'react-router-dom'

import { PortalCheckout } from '@/pages/portal/checkout'

export const portalRoutes: RouteObject = {
  children: [
    {
      path: 'checkout',
      children: [
        {
          index: true,
          element: <PortalCheckout />,
        },
      ],
    },
  ],
}
