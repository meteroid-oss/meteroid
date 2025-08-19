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
import { OauthSuccess } from "@/pages/auth/oauth-success";

export const anonymousRoutes: RouteObject = {
  element: <AnonymousRoutes/>,
  children: [
    {
      element: <AuthLayout/>,
      children: [
        {
          element: <AuthFormLayout/>,
          children: [
            {
              path: '/login',
              element: <Login/>,
            },
            {
              path: '/registration',
              element: <Registration/>,
            },
          ],
        },
        {
          path: '/check-inbox',
          element: <CheckInbox/>,
        },
        {
          path: '/validate-email',
          element: <ValidateEmail/>,
        },
        {
          path: '/forgot-password',
          element: <ForgotPassword/>,
        },
        {
          path: '/check-inbox-password',
          element: <CheckInboxPassword/>,
        },
        {
          path: '/reset-password',
          element: <ResetPassword/>,
        },
        {
          path: '/oauth_success',
          element: <OauthSuccess/>
        }
      ],
    },
  ],
}
