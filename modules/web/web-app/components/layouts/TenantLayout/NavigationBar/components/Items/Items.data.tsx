import { colors } from '@md/foundation'
import {
  CatalogIcon,
  CustomersIcon,
  EventsIcon,
  HomeIcon,
  InvoicesIcon,
  ReportsIcon,
  SubscriptionsIcon,
} from '@md/icons'
import { GaugeIcon, LightbulbIcon } from 'lucide-react'

import { NavigationItemType } from './components/Item/Item.types'

export const NAVIGATION_ITEMS: NavigationItemType[] = [
  {
    label: 'Home',
    to: '.',
    end: true,
    icon: <HomeIcon size={20} stroke={colors.primary9} />,
    divider: true,
  },
  {
    label: 'Customers',
    to: 'customers',
    icon: <CustomersIcon size={20} />,
  },
  {
    label: 'Subscriptions',
    to: 'subscriptions',
    icon: <SubscriptionsIcon size={20} fill={colors.primary9} />,
  },
  {
    label: 'Invoices, credit notes & quotes',
    to: 'invoices',
    icon: <InvoicesIcon size={20} stroke={colors.primary9} />,
    divider: true,
  },
  {
    label: 'Plans & pricing', // metrics
    to: 'billing',
    icon: <CatalogIcon size={20} fill={colors.primary9} />,
  },
  {
    label: 'Metrics',
    to: 'metrics', // TODO
    icon: <GaugeIcon size={20} />,
  },
  {
    label: 'Growth',
    to: 'growth',
    icon: <LightbulbIcon size={20} />,
    divider: true,
  },
  {
    label: 'Reports',
    to: 'reports',
    icon: <ReportsIcon size={20} fill={colors.primary9} />,
  },
  {
    label: 'Logs',
    to: 'logs',
    icon: <EventsIcon size={20} stroke={colors.primary9} />,
  },
]
