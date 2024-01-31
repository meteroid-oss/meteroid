import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { Loading } from '@/components/atoms/Loading'
import { TenantPageLayout } from '@/components/layouts'
import SidebarMenu from '@/components/organisms/SidebarMenu'
import ProductEmptyState from '@/features/productCatalog/ProductEmptyState'
import { useQuery, useMutation, createConnectQueryKey } from '@connectrpc/connect-query'
import {
  createProductFamily,
  listProductFamilies,
} from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { useQueryClient } from '@tanstack/react-query'

export const Billing: FunctionComponent = () => {
  const families = useQuery(listProductFamilies)

  if (families.isLoading) return <Loading />
  if (!families.data?.productFamilies?.length) return <FamilyCreationModalPage />
  return <Navigate to={families.data?.productFamilies[0].externalId} />
}

export const BillingOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout
      title="Billing"
      innerMenu={
        <SidebarMenu
          items={[
            {
              label: 'Pricing items',
              items: [
                {
                  label: 'Packages',
                  to: 'packages',
                },
                {
                  label: 'Plans',
                  to: 'plans',
                },
                {
                  label: 'Add-ons',
                  to: 'addons',
                },
                {
                  label: 'Credits',
                  to: 'credits',
                },
                {
                  label: 'Coupons',
                  to: 'coupons',
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
                  label: 'Custom Pricing units',
                  to: 'units',
                },
                {
                  label: 'Billing Frequencies',
                  to: 'frequencies',
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

export const FamilyCreationModalPage = () => {
  const queryClient = useQueryClient()

  const createDefaultMutation = useMutation(createProductFamily, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listProductFamilies) })
    },
  })

  const createDefault = () =>
    createDefaultMutation.mutateAsync({ name: 'Default', externalId: 'default' })

  return (
    <TenantPageLayout title="Product Billing">
      <div className="storage-container flex flex-grow">
        <ProductEmptyState
          title="Product Families"
          ctaButtonLabel="Create default" // TODO modal
          onClickCta={createDefault}
        >
          <p className="text-scale-1100 text-sm">
            Create a Product Family to categorize and isolate your products and plans.
          </p>
          <p className="text-scale-1100 text-sm">
            Product Families allow for complex multi-services setup. For most cases, a single
            default family is enough.
          </p>
        </ProductEmptyState>
      </div>
    </TenantPageLayout>
  )
}
