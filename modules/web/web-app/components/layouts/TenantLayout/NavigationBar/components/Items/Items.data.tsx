import { BillingIcon, Catalog2Icon, CustomersIcon, HomeIcon, ReportsIcon } from '@md/icons'
import { LightbulbIcon } from 'lucide-react'

import { NavigationItemType } from './components/Item/Item.types'

export const NAVIGATION_ITEMS: NavigationItemType[] = [
  {
    label: 'Home',
    to: '.',
    end: true,
    icon: <HomeIcon size={20} />,
    divider: true,
  },

  {
    label: 'Product catalog', // metrics
    to: 'plans',
    icon: <Catalog2Icon size={18} />,
  },
  {
    label: 'Billing',
    to: 'billing',
    icon: <BillingIcon size={22} className="ml-[-2px]" />,
  },
  {
    label: 'Customers',
    to: 'customers',
    icon: <CustomersIcon size={20} />,
    divider: true,
  },
  {
    label: 'Growth',
    to: 'growth',
    icon: <LightbulbIcon size={20} />,
  },
  {
    label: 'Reports',
    to: 'reports',
    icon: <ReportsIcon size={20} />,
  },
]
