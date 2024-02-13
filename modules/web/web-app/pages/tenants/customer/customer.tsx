import { spaces } from '@md/foundation'
import { Flex, Skeleton } from '@ui/components'
import { ChevronLeftIcon, LockIcon } from 'lucide-react'
import { Fragment } from 'react'
import { useNavigate } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'
import { AddressCard } from '@/features/customers/cards/address/AddressCard'
import { BalanceCard } from '@/features/customers/cards/balance/BalanceCard'
import { InvoicesCard } from '@/features/customers/cards/balance/InvoicesCard'
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
              <Flex direction="column" fullHeight>
                <CustomerCard customer={data} />
                <AddressCard customer={data} />
                <BalanceCard customer={data} />
                <InvoicesCard customer={data} />
              </Flex>
            </>
          )}
        </Flex>
      </TenantPageLayout>
    </Fragment>
  )
}
