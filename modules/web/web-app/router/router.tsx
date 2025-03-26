import { createBrowserRouter } from 'react-router-dom'

import { OnboardingLayout } from '@/components/layouts/OnboardingLayout'
import { ProtectedRoutes } from '@/features/auth/sessionRoutes'
import { Logout } from '@/pages/auth'
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
              path: '/onboarding',
              element: <OnboardingLayout />,
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
