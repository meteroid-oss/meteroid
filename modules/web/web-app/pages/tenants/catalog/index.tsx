import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { Loading } from '@/components/atoms/Loading'
import { TenantPageLayout } from '@/components/layouts'
import SidebarMenu from '@/components/organisms/SidebarMenu'
import { useQuery } from '@/lib/connectrpc'
import { FamilyCreationModalPage } from '@/pages/tenants/billing'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const Catalog: FunctionComponent = () => {
  const families = useQuery(listProductFamilies)

  if (families.isLoading) return <Loading />
  if (!families.data?.productFamilies?.length) return <FamilyCreationModalPage />
  return <Navigate to={families.data?.productFamilies[0].externalId} />
}

export const CatalogOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout
      title="Product Catalog"
      innerMenu={
        <SidebarMenu
          items={[
            {
              label: 'Catalog',
              items: [
                {
                  label: 'Product Items',
                  to: 'items',
                },
                {
                  label: 'Data catalog (?)',
                  to: 'data-catalog',
                },
                {
                  label: 'Usage Metrics',
                  to: 'metrics',
                },
                {
                  label: 'Cost Metrics',
                  to: 'todo',
                },
                {
                  label: 'Features',
                  to: 'features',
                },
              ],
            },
            {
              label: 'Configuration',
              items: [
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
