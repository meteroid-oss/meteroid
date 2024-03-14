import { useMutation, createConnectQueryKey } from '@connectrpc/connect-query'
import { Dot } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FunctionComponent } from 'react'
import { Navigate, Outlet } from 'react-router-dom'

import SidebarMenu from '@/components/SidebarMenu'
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
    <TenantPageLayout
      title="Billing"
      innerMenu={
        <SidebarMenu
          items={[
            {
              label: 'Subscriptions',
              items: [
                {
                  label: 'Active',
                  to: 'subscriptions',
                },
                {
                  label: (
                    <span className="flex  items-center gap-2 pl-2 my-[-2px]">
                      <Dot className="text-success h-2" />
                      <>Trials</>
                    </span>
                  ),
                  to: 'subscriptions/trials',
                },
                {
                  label: (
                    <span className="flex items-center gap-2 pl-2 my-[-2px]">
                      <Dot className="text-destructive h-2" />
                      <>At risk</>
                    </span>
                  ),
                  to: 'subscriptions/past-due',
                },
                {
                  label: 'Expired',
                  to: 'subscriptions/expired',
                },
                {
                  label: 'Cancelled',
                  to: 'subscriptions/cancelled',
                },
              ],
            },
            {
              label: 'Invoicing',
              items: [
                {
                  label: 'Invoices',
                  to: 'invoices',
                },
                {
                  label: (
                    <span className="flex  items-center gap-2 pl-2 my-[-2px]">
                      <Dot className="text-muted-foreground h-2" />
                      <>Drafts</>
                    </span>
                  ),
                  to: 'subscriptions/trials',
                },
                {
                  label: (
                    <span className="flex  items-center gap-2 pl-2 my-[-2px]">
                      <Dot className="text-brand h-2" />
                      <>Pending</>
                    </span>
                  ),
                  to: 'subscriptions/trials',
                },
                {
                  label: (
                    <span className="flex items-center gap-2 pl-2 my-[-2px]">
                      <Dot className="text-warning h-2" />
                      <>Past due</>
                    </span>
                  ),
                  to: 'subscriptions/past-due',
                },
                {
                  label: 'Credit notes',
                  to: 'credit-notes',
                },
                {
                  label: 'Quotes',
                  to: 'quotes',
                },
              ],
            },
            {
              label: 'Cost center',
              items: [
                {
                  label: 'Alerts',
                  to: 'cost-alerts',
                },
              ],
            },
            {
              label: 'Configuration',
              items: [
                {
                  label: 'Invoice configuration',
                  to: 'invoice-config',
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
