import { createBrowserRouter } from 'react-router-dom'

import { ProtectedRoutes } from '@/features/auth/sessionRoutes'
import { Logout } from '@/pages/auth'
import { Root } from '@/pages/root'
import { Tenants } from '@/pages/tenants'
import { Providers } from 'providers/Providers'
import { anonymousRoutes } from 'router/anonymous'
import { tenantRoutes } from 'router/tenant'

const router = createBrowserRouter(
  [
    {
      element: <Providers />,
      children: [
        {
          path: '/',
          element: <ProtectedRoutes />,
          children: [
            {
              index: true,
              element: <Root />,
            },
            tenantRoutes,
            // tenant list
            {
              path: '/tenants',
              element: <Tenants />,
            },
          ],
        },
        anonymousRoutes,
        {
          path: '/logout',
          element: <Logout />,
        },
      ],
    },
  ],
  {
    future: {
      v7_normalizeFormMethod: true,
    },
  }
)

export default router
