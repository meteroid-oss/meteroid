import { RouteObject } from 'react-router-dom'

import { NotImplemented } from '@/features/NotImplemented'
import { Growth, GrowthOutlet } from '@/pages/tenants/growth'

export const growthRoutes: RouteObject = {
  path: 'growth',
  element: <GrowthOutlet />,
  children: [
    {
      index: true,
      element: <Growth />,
    },
    {
      path: '*',
      element: <NotImplemented />,
    },
  ],
}
