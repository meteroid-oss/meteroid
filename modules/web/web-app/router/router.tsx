import { createBrowserRouter } from 'react-router-dom'

import { OnboardingLayout } from '@/components/layouts/OnboardingLayout'
import { ProtectedRoutes } from '@/features/auth/sessionRoutes'
import { Logout } from '@/pages/auth'
import { AcceptInvite, InviteAuthenticated } from '@/pages/invite'
import { InviteOnboarding, OrganizationOnboarding, UserOnboarding } from '@/pages/onboarding'
import { OrganizationRoot } from '@/pages/organizationRoot'
import { Root } from '@/pages/root'
import { TenantNew } from '@/pages/tenants/new'
import { Providers } from 'providers/Providers'
import { anonymousRoutes } from 'router/anonymous'
import { portalRoutes } from 'router/portal'
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
            {
              path: '/invite-authenticated',
              element: <InviteAuthenticated />,
              handle: { title: 'Accept invite' },
            },
            {
              path: '/onboarding',
              element: <OnboardingLayout />,
              handle: { title: 'Onboarding' },
              children: [
                {
                  path: 'user',
                  element: <UserOnboarding />,
                },
                {
                  path: 'organization',
                  element: <OrganizationOnboarding />,
                },
                {
                  path: 'invite',
                  element: <InviteOnboarding />,
                },
              ],
            },
            {
              path: '/:organizationSlug',
              children: [
                {
                  index: true,
                  element: <OrganizationRoot />,
                },
                tenantRoutes,
                {
                  path: 'tenants/new',
                  element: <TenantNew />,
                  handle: { title: 'New tenant' },
                },

              ],
            },
          ],
        },
        anonymousRoutes,
        portalRoutes,
        {
          path: '/logout',
          element: <Logout />,
          handle: { title: 'Logout' },
        },
        {
          path: '/invite',
          element: <AcceptInvite />,
          handle: { title: 'Accept invite' },
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
