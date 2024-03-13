import { spaces } from '@md/foundation'
import { Flex } from '@ui2/components/legacy'
import { Skeleton, Tabs, TabsContent, TabsList, TabsTrigger } from '@ui2/components'
import { ChevronLeftIcon, LockIcon } from 'lucide-react'
import { Fragment } from 'react'
import { useNavigate } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'
import { InvoicesCard } from '@/features/customers/cards/InvoicesCard'
import { SubscriptionsCard } from '@/features/customers/cards/SubscriptionsCard'
import { AddressCard } from '@/features/customers/cards/address/AddressCard'
import { BalanceCard } from '@/features/customers/cards/balance/BalanceCard'
import { CustomerCard } from '@/features/customers/cards/customer/CustomerCard'
import { useQuery } from '@/lib/connectrpc'
import { getCustomer } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Customer = () => {
  const navigate = useNavigate()
  const { customerId } = useTypedParams<{ customerId: string }>()
  const customerQuery = useQuery(
    getCustomer,
    {
      id: customerId ?? '',
    },
    { enabled: Boolean(customerId) }
  )

  const data = customerQuery.data
  const isLoading = customerQuery.isLoading

  return (
    <Fragment>
      <TenantPageLayout title="Customer">
        <Flex direction="column" gap={spaces.space9} fullHeight>
          {isLoading || !data ? (
            <>
              <Skeleton height={16} width={50} />
              <Skeleton height={44} />
            </>
          ) : (
            <>
              <div className="flex justify-between">
                <div className="flex gap-2 items-center text-2xl">
                  <ChevronLeftIcon
                    className="font-semibold cursor-pointer"
                    onClick={() => navigate('..')}
                  />
                  <h2 className="font-semibold">
                    {data.alias || data.name}
                    <div className="text-sm font-light text-slate-500">{data.email}</div>
                  </h2>
                </div>
                {data.archivedAt && (
                  <div className="text-sm">
                    <LockIcon />
                  </div>
                )}
              </div>
              <div className="grid grid-cols-3 gap-x-6">
                <CustomerCard customer={data} className="col-span-2" />
                <BalanceCard customer={data} className="col-span-1" />
                <AddressCard customer={data} className="col-span-2" />

                <Tabs defaultValue="invoices" className="w-full col-span-3">
                  <TabsList className="w-full justify-start">
                    <TabsTrigger value="invoices">Invoices</TabsTrigger>
                    <TabsTrigger value="subscriptions">Subscriptions</TabsTrigger>
                  </TabsList>
                  <TabsContent value="invoices" className="pt-4">
                    <InvoicesCard customer={data} />
                  </TabsContent>
                  <TabsContent value="subscriptions" className="pt-4">
                    <SubscriptionsCard customer={data} />
                  </TabsContent>
                </Tabs>
              </div>
            </>
          )}
        </Flex>
      </TenantPageLayout>
    </Fragment>
  )
}
