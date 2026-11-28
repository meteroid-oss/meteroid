import { Tabs, TabsList, TabsTrigger } from '@md/ui'
import { Link } from 'react-router'

import { slotItems } from '@/edition'
import { useBasePath } from '@/hooks/useBasePath'

/** Offers, Coupons, then the tabs modules add (`offers.tabs`), each its own page. */
export const OffersTabs = ({ active }: { active: 'offers' | 'coupons' }) => {
  const basePath = useBasePath()
  return (
    <Tabs value={active}>
      <TabsList variant="ghost">
        <TabsTrigger value="offers" asChild>
          <Link to={`${basePath}/offers`}>Offers</Link>
        </TabsTrigger>
        <TabsTrigger value="coupons" asChild>
          <Link to={`${basePath}/offers/coupons`}>Coupons</Link>
        </TabsTrigger>
        {slotItems('offers.tabs').map(tab => (
          <TabsTrigger key={tab.path} value={tab.path} asChild>
            <Link to={`${basePath}/${tab.path}`}>{tab.label}</Link>
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  )
}
