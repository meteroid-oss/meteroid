import { RouteObject } from 'react-router-dom'

import { Subscriptions } from '@/pages/tenants/subscription'
import { SubscriptionCreate } from '@/pages/tenants/subscription/subscriptionCreate'

export const subscriptionRoutes: RouteObject = {
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
}
