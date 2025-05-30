import { spaces } from '@md/foundation'
import { PaginationState } from '@tanstack/react-table'
import { Skeleton } from '@ui/components'
import { Flex } from '@ui/components/legacy'
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
    pageSize: 20,
  })

  const invoicesQuery = useQuery(listSubscriptions, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    customerId: customer.id,
  })

  return invoicesQuery.isLoading ? (
    <Flex direction="column" gap={spaces.space9} fullHeight>
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </Flex>
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
