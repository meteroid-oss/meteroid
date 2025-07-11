import { Box, ChartPie, Flag, Home, LifeBuoy, ReceiptText, Settings, User } from 'lucide-react'

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
          disabled: true,
        },
        {
          title: 'Features',
          url: 'features',
          disabled: true,
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
          disabled: true,
        },
      ],
    },
    {
      title: 'Customers',
      url: 'customers',
      icon: User,
    },
    {
      title: 'Insights',
      url: 'insights',
      icon: Flag,
      disabled: true,
    },
    {
      title: 'Reports',
      url: 'reports',
      icon: ChartPie,
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
