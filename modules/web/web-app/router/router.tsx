import { createBrowserRouter } from 'react-router-dom'

import { ProtectedRoutes } from '@/features/auth/sessionRoutes'
import { Logout } from '@/pages/auth'
import { Root } from '@/pages/root'
import { TenantNew } from '@/pages/tenants/new'
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
            {
              path: '/tenants/new',
              element: <TenantNew />,
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
