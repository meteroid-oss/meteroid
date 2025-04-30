import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'
import ProductEmptyState from '@/features/productCatalog/ProductEmptyState'
import {
  createProductFamily,
  listProductFamilies,
} from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const Billing: FunctionComponent = () => {
  return <Navigate to="subscriptions" />
}

export const BillingOutlet: FunctionComponent = () => {
  return (
    <TenantPageLayout>
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

  const createDefault = () => createDefaultMutation.mutateAsync({ name: 'Default' })

  return (
    <TenantPageLayout>
      <div className="storage-container flex flex-grow">
        <ProductEmptyState
          title="Product Families"
          ctaButtonLabel="Create default" // TODO modal
          onClickCta={createDefault}
        >
          <p className="text-muted-foreground text-sm">
            Create a Product Family to categorize and isolate your products and plans.
          </p>
          <p className="text-muted-foreground text-sm">
            Product Families allow for complex multi-services setup. For most cases, a single
            default family is enough.
          </p>
        </ProductEmptyState>
      </div>
    </TenantPageLayout>
  )
}
