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
          title: 'Plan',
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
          title: 'Features',
          url: 'features',
          isActive: false,
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
          title: 'Credit notes',
          url: 'credit-notes',
        },
        {
          title: 'Quotes',
          url: 'quotes',
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
