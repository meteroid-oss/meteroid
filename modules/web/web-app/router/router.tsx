import { createBrowserRouter, type RouteObject } from 'react-router'

import { OnboardingLayout } from '@/components/layouts/OnboardingLayout'
import { editionOrgRoutes, editionProtectedRoutes, editionRootRoutes } from '@/edition'
import { ProtectedRoutes } from '@/features/auth/sessionRoutes'
import { Logout } from '@/pages/auth'
import { AcceptInvite, InviteAuthenticated } from '@/pages/invite'
import {
  DemoDataOnboarding,
  InviteOnboarding,
  OrganizationOnboarding,
  UserOnboarding,
} from '@/pages/onboarding'
import { OrganizationRoot } from '@/pages/organizationRoot'
import { Root } from '@/pages/root'
import { TenantNew } from '@/pages/tenants/new'
import { Providers } from 'providers/Providers'
import { anonymousRoutes } from 'router/anonymous'
import { portalRoutes } from 'router/portal'
import { tenantRoutes } from 'router/tenant'

// Design previews, served by the dev server only (and left out of production builds).
const devPreviewRoutes: RouteObject[] = import.meta.env.DEV
  ? [
      {
        path: '/preview/go-live',
        lazy: async () => ({
          Component: (await import('@/pages/preview/goLivePreview')).GoLivePreview,
        }),
        handle: { title: 'Go live preview' },
      },
    ]
  : []

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
            ...editionProtectedRoutes(),
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
                {
                  path: 'demo/:organizationSlug',
                  element: <DemoDataOnboarding />,
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
                ...editionOrgRoutes(),
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
        ...editionRootRoutes(),
        ...devPreviewRoutes,
      ],
    },
  ]
)

export default router
