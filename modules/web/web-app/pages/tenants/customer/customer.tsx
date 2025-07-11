import { Card, Flex, Separator, Skeleton } from '@md/ui'
import { ChevronDown, Plus } from 'lucide-react'
import { Fragment, useState } from 'react'

import { TenantPageLayout } from '@/components/layouts'
import { CustomerHeader, CustomersEditPanel } from '@/features/customers'
import { InvoicesCard } from '@/features/customers/cards/InvoicesCard'
import { SubscriptionsCard } from '@/features/customers/cards/SubscriptionsCard'
import { CustomerInvoiceModal } from '@/features/customers/modals/CustomerInvoiceModal'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Customer = () => {
  const { customerId } = useTypedParams<{ customerId: string }>()

  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [createInvoiceVisible, setCreateInvoiceVisible] = useState(false)

  const customerQuery = useQuery(
    getCustomerById,
    {
      id: customerId ?? '',
    },
    { enabled: Boolean(customerId) }
  )

  const data = customerQuery.data?.customer
  const isLoading = customerQuery.isLoading

  return (
    <Fragment>
      <TenantPageLayout>
        <Flex direction="column" className="h-full">
          <CustomerHeader
            setEditPanelVisible={setEditPanelVisible}
            name={data?.name || data?.alias}
            // setShowIncoice={() => setCreateInvoiceVisible(true)}
            setShowIncoice={() => false}
          />
          {isLoading || !data ? (
            <>
              <Skeleton height={16} width={50} />
              <Skeleton height={44} />
            </>
          ) : (
            <Flex className="h-full">
              <Flex direction="column" className="gap-4 w-2/3 border-r border-border px-12 py-6">
                <div className="text-lg font-medium">Overview</div>
                <div className="grid grid-cols-2 gap-x-4">
                  <OverviewCard title="MRR" value={undefined} />
                  <OverviewCard
                    title="Balance"
                    value={data?.balanceValueCents ? Number(data.balanceValueCents) : undefined}
                  />
                </div>
                <Flex align="center" justify="between" className="mt-4">
                  <div className="text-lg font-medium">Subscriptions</div>
                  <Flex align="center" className="gap-1 text-sm">
                    <Plus size={10} /> Assign subscription
                  </Flex>
                </Flex>
                <div className="flex-none">
                  <SubscriptionsCard customer={data} />
                </div>
                <Flex align="center" justify="between" className="mt-4">
                  <div className="text-lg font-medium">Invoices</div>
                  <Flex align="center" className="gap-1 text-sm">
                    <Plus size={10} /> Create invoice
                  </Flex>
                </Flex>
                <div className="flex-none">
                  <InvoicesCard customer={data} />
                </div>
              </Flex>
              <Flex direction="column" className="gap-2 w-1/3">
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-lg font-medium">{data.name}</div>
                  <div className="text-muted-foreground text-[13px] mb-3">{data.alias}</div>
                  <FlexDetails title="Legal name" value={data.name} />
                  <FlexDetails title="Email" value={data.billingEmail} />
                  <FlexDetails title="Currency" value={data.currency} />
                  <FlexDetails title="Country" value={data.billingAddress?.country ?? ''} />
                  <Flex align="center" justify="between">
                    <div className="text-[13px] text-muted-foreground">Address</div>
                    <div className="text-[13px]">{data.billingAddress?.city}</div>
                  </Flex>
                  <FlexDetails title="Tax rate" value="None" />
                  <FlexDetails title="Tax ID" value="None" />
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Integrations</div>
                  <FlexDetails title="Alias (External ID)" value={data.alias} />
                  {/* TODO <FlexDetails title="Hubspot ID" value={data.connectionMetadata?.hubspot?.0?.externalId} /> */}
                  <FlexDetails title="Stripe ID" value="N/A" />
                </Flex>
                <Separator className="-my-3" />
                <Flex direction="column" className="gap-2 p-6">
                  <div className="text-[15px] font-medium">Payment</div>
                  <FlexDetails
                    title="Payment method"
                    value={data.currentPaymentMethodId ?? 'None'}
                  />
                  <FlexDetails title="Payment term" value="N/A" />
                  <FlexDetails title="Grace period" value="None" />
                </Flex>
              </Flex>
            </Flex>
          )}
        </Flex>
      </TenantPageLayout>
      <CustomersEditPanel
        visible={editPanelVisible}
        closePanel={() => setEditPanelVisible(false)}
      />
      <CustomerInvoiceModal openState={[createInvoiceVisible, setCreateInvoiceVisible]} />
    </Fragment>
  )
}

const OverviewCard = ({ title, value }: { title: string; value?: number }) => (
  <Card className="bg-[#1A1A1A] bg-gradient-to-t from-[rgba(243,242,241,0.00)] to-[rgba(243,242,241,0.02)] rounded-md p-5">
    <Flex align="center" className="gap-1 text-muted-foreground">
      <div className="text-[13px]">{title}</div>
      <ChevronDown size={10} className="mt-0.5" />
    </Flex>
    <div className="mt-4 text-xl">€ {value}</div>
  </Card>
)

const FlexDetails = ({ title, value }: { title: string; value?: string }) => (
  <Flex align="center" justify="between">
    <div className="text-[13px] text-muted-foreground">{title}</div>
    <div className="text-[13px]">{value ?? 'N/A'}</div>
  </Flex>
)
