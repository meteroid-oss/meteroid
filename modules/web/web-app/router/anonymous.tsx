import { RouteObject } from 'react-router-dom'

import { Login, Registration } from '@/features/auth'
import AuthPageTemplate from '@/features/auth/components/AuthPageTemplate'
import { AnonymousRoutes } from '@/features/auth/sessionRoutes'
import { Recovery, Verification } from '@/pages'


export const anonymousRoutes: RouteObject = {
  element: <AnonymousRoutes />,
  children: [
    {
      element: <AuthPageTemplate />,
      children: [
        {
          path: '/login',
          element: <Login />,
        },
        {
          path: '/registration',
          element: <Registration />,
        },
      ],
    },
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
