import { Box, Home, LifeBuoy, ReceiptText, Settings, Sparkles, User, Zap } from 'lucide-react'

import { editionId } from '@/edition'
import { env } from '@/lib/env'

import type { NavMainItem, NavMainSection, NavSecondaryItem } from '@/edition/types'

// Read permission a top-level nav section requires. Sections without an entry
// (Home) are always shown. Sub-items inherit their parent's permission unless
// listed in NAV_SUB_READ_PERMISSION. Mirrors the backend RBAC classification.
const NAV_READ_PERMISSION: Record<string, string> = {
  'Product catalog': 'catalog:read',
  Customers: 'customers:read',
  Events: 'events:read',
}

// Billing spans several permissions, so its sub-items are gated individually and
// the section shows while at least one remains.
const NAV_SUB_READ_PERMISSION: Record<string, string> = {
  Subscriptions: 'billing:subscriptions:read',
  Invoices: 'billing:invoices:read',
  Quotes: 'billing:quotes:read',
  'Credit notes': 'billing:invoices:read',
  Wallets: 'billing:invoices:read',
  Accounts: 'alerts:read',
}

// The open-source edition presents what the Enterprise images add on one page, listed above
// "What's new".
const upgradeNav = () =>
  editionId === 'oss' ? [{ title: 'Upgrade', url: 'settings/upgrade', icon: Sparkles }] : []

export const getFilteredSidebarItems = (
  isExpress: boolean,
  canView: (permission: string) => boolean = () => true,
  moduleNavSecondary: NavSecondaryItem[] = [],
  moduleNavMain: (NavMainSection & { counts: Record<string, number> })[] = []
) => {
  const byPermission = (item: { title: string }) => {
    const required = NAV_READ_PERMISSION[item.title]
    return !required || canView(required)
  }
  const bySubPermission = (item: { title: string }) => {
    const required = NAV_SUB_READ_PERMISSION[item.title]
    return !required || canView(required)
  }
  if (!isExpress) {
    const byItemPermission = (item: NavMainItem) => !item.permission || canView(item.permission)
    const coreNav = sidebarItems.mainNav.filter(byPermission).map(item =>
      'items' in item && item.items
        ? {
            ...item,
            items: item.items.filter(bySubPermission),
          }
        : item
    )
    const mainNav = coreNav
      .flatMap(item => [
        item,
        ...moduleNavMain
          .filter(section => section.after === item.title)
          .map(({ title, icon, new: isNew, items, counts }) => ({
            title,
            icon,
            new: isNew,
            items: items.filter(byItemPermission).map(({ permission: _, ...sub }) => ({
              ...sub,
              count: counts[sub.title],
            })),
          })),
      ])
      // A section whose sub-items are all gated out has nothing to open.
      .filter(item => !('items' in item && item.items) || item.items.length > 0)
    return {
      ...sidebarItems,
      mainNav,
      navSecondary: [
        ...sidebarItems.navSecondary,
        ...moduleNavSecondary.map(({ title, url, icon }) => ({ title, url, icon })),
      ],
      navLeading: upgradeNav(),
    }
  }

  return {
    mainNav: [
      {
        title: 'Home',
        url: '.',
        icon: Home,
      },
      {
        title: 'Product catalog',
        icon: Box,
        items: [
          {
            title: 'Plans',
            url: 'plans',
          },
          {
            title: 'Metrics',
            url: 'metrics',
          },
        ],
      },
      {
        title: 'Billing',
        icon: ReceiptText,
        items: [
          {
            title: 'Subscriptions',
            url: 'subscriptions',
          },
          {
            title: 'Invoices',
            url: 'invoices',
          },
        ],
      },
      {
        title: 'Customers',
        url: 'customers',
        icon: User,
      },
    ],
    navSecondary: [
      {
        title: 'Help & Feedback',
        url: 'help',
        icon: LifeBuoy,
      },
      {
        title: 'Settings',
        url: 'settings',
        icon: Settings,
      },
    ],
    navLeading: upgradeNav(),
  }
}

export const sidebarItems = {
  mainNav: [
    {
      title: 'Home',
      url: '.',
      icon: Home,
    },
    {
      title: 'Product catalog',
      icon: Box,
      items: [
        {
          title: 'Plans',
          url: 'plans',
        },
        {
          title: 'Products',
          url: 'items',
        },

        {
          title: 'Metrics',
          url: 'metrics',
        },
        {
          title: 'Coupons',
          url: 'coupons',
        },
        {
          title: 'Addons',
          url: 'addons',
        },
        {
          title: 'Features',
          url: 'features',
          disabled: !env.entitlementsEnabled,
        },
      ],
    },
    {
      title: 'Billing',
      icon: ReceiptText,
      items: [
        {
          title: 'Subscriptions',
          url: 'subscriptions',
        },
        {
          title: 'Invoices',
          url: 'invoices',
        },
        {
          title: 'Quotes',
          url: 'quotes',
        },

        {
          title: 'Credit notes',
          url: 'credit-notes',
        },
        {
          title: 'Wallets',
          url: 'wallets',
        },
      ],
    },
    {
      title: 'Customers',
      url: 'customers',
      icon: User,
    },

    {
      title: 'Events',
      url: 'events',
      icon: Zap,
    },
  ],
  navSecondary: [
    {
      title: 'Help & Feedback',
      url: 'help',
      icon: LifeBuoy,
    },
    {
      title: 'Settings',
      url: 'settings',
      icon: Settings,
    },
  ],
}
