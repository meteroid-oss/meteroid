import { FunctionComponent } from 'react'
import { Outlet } from 'react-router-dom'

import SidebarMenu from '@/components/SidebarMenu'
import { TenantPageLayout } from '@/components/layouts'

export const CatalogOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout
      title="Offering"
      innerMenu={
        <SidebarMenu
          items={[
            {
              label: 'Pricing',
              items: [
                {
                  label: 'Plans',
                  to: 'plans',
                },
                {
                  label: 'Packages',
                  to: 'packages',
                },
                {
                  label: 'Add-ons',
                  to: 'addons',
                },
                {
                  label: 'Credits',
                  to: 'credits',
                  disabled: true,
                },
                {
                  label: 'Coupons',
                  to: 'coupons',
                },
              ],
            },
            {
              label: 'Product catalog',
              items: [
                {
                  label: 'Products',
                  to: 'items',
                },
                {
                  label: 'Metrics',
                  to: 'metrics',
                  // TODO USage / Cost tabs
                },

                {
                  label: 'Features',
                  to: 'features',
                  disabled: true,
                },
              ],
            },
            {
              label: 'Configuration',
              items: [
                {
                  label: 'Currencies',
                  to: 'currencies',
                },
                {
                  label: 'Custom units',
                  to: 'units',
                },
              ],
            },
          ]}
        />
      }
    >
      <Outlet />
    </TenantPageLayout>
  )
}
