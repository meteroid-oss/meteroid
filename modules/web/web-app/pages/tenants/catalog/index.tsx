import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import SidebarMenu from '@/components/SidebarMenu'
import { TenantPageLayout } from '@/components/layouts'
import { useQuery } from '@/lib/connectrpc'
import { FamilyCreationModalPage } from '@/pages/tenants/billing'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const Catalog: FunctionComponent = () => {
  const families = useQuery(listProductFamilies)

  if (families.isLoading) return <Loading />
  if (!families.data?.productFamilies?.length) return <FamilyCreationModalPage />
  return <Navigate to={`${families.data?.productFamilies[0].localId}/plans`} />
}

export const CatalogOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout
      title="Product Catalog"
      familyPicker
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
              label: 'Products',
              items: [
                {
                  label: 'Product Items',
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
