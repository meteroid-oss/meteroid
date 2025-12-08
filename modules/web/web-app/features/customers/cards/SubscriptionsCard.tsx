import { PaginationState } from '@tanstack/react-table'
import { Skeleton } from '@ui/components'
import { useState } from 'react'

import { SubscriptionsTable } from '@/features/subscriptions/SubscriptionsTable'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

type Props = {
  customer: Customer
}

export const SubscriptionsCard = ({ customer }: Props) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 5,
  })

  const invoicesQuery = useQuery(listSubscriptions, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    customerId: customer.id,
    status: [],
  })

  return invoicesQuery.isLoading ? (
    <div className="flex flex-col gap-8 h-full">
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </div>
  ) : (
    <SubscriptionsTable
      data={invoicesQuery.data?.subscriptions || []}
      totalCount={Number(invoicesQuery.data?.paginationMeta?.totalItems || 0)}
      pagination={pagination}
      setPagination={setPagination}
      isLoading={invoicesQuery.isLoading}
      hideCustomer
    />
  )
}
