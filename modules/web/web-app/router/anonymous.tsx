import { RouteObject } from 'react-router-dom'

import { AnonymousRoutes } from '@/features/auth/sessionRoutes'
import { Login, Recovery, Registration, Verification } from '@/pages'

export const anonymousRoutes: RouteObject = {
  element: <AnonymousRoutes />,
  children: [
    {
      path: '/login',
      element: <Login />,
    },
    {
      path: '/registration',
      element: <Registration />,
    },
    {
      path: '/recovery',
      element: <Recovery />,
    },
    {
      path: '/verification',
      element: <Verification />,
    },
  ],
}
