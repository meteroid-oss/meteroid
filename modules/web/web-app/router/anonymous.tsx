import { RouteObject } from 'react-router-dom'

import { AuthFormLayout } from '@/features/auth/components/AuthFormLayout'
import { AuthLayout } from '@/features/auth/components/AuthLayout'
import { AnonymousRoutes } from '@/features/auth/sessionRoutes'
import {
  CheckInbox,
  CheckInboxPassword,
  ForgotPassword,
  Login,
  Registration,
  ResetPassword,
  ValidateEmail,
} from '@/pages/auth'
import { OauthSuccess } from '@/pages/auth/oauth-success'

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
              handle: { title: 'Sign in' },
            },
            {
              path: '/registration',
              element: <Registration />,
              handle: { title: 'Sign up' },
            },
          ],
        },
        {
          path: '/check-inbox',
          element: <CheckInbox />,
          handle: { title: 'Check your inbox' },
        },
        {
          path: '/validate-email',
          element: <ValidateEmail />,
          handle: { title: 'Validate email' },
        },
        {
          path: '/forgot-password',
          element: <ForgotPassword />,
          handle: { title: 'Forgot password' },
        },
        {
          path: '/check-inbox-password',
          element: <CheckInboxPassword />,
          handle: { title: 'Check your inbox' },
        },
        {
          path: '/reset-password',
          element: <ResetPassword />,
          handle: { title: 'Reset password' },
        },
        {
          path: '/oauth_success',
          element: <OauthSuccess />,
          handle: { title: 'Sign in' },
        },
      ],
    },
  ],
}
