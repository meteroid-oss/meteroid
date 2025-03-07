import { Login, Registration } from '@/features/auth'
import { AuthFormLayout } from '@/features/auth/components/AuthFormLayout'
import { AuthLayout } from '@/features/auth/components/AuthLayout'
import { AnonymousRoutes } from '@/features/auth/sessionRoutes'
import { PasswordCreation } from '@/pages/auth/password-creation'
import { ValidateEmail } from '@/pages/auth/validate-email'
import { RouteObject } from 'react-router-dom'

export const anonymousRoutes: RouteObject = {
  element: <AnonymousRoutes />,
  children: [
    {
      element: <AuthLayout />,
      children: [
        {
          element: <AuthFormLayout />,
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
          path: '/validate-email',
          element: <ValidateEmail />,
        },
        {
          path: '/password-creation',
          element: <PasswordCreation />,
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
      path: '/validate-email',
      element: <ValidateEmail />,
    },
    {
      path: '/password-creation',
      element: <PasswordCreation />,
    },
  ],
}
