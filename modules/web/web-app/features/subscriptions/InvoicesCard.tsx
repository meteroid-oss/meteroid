import { spaces } from '@md/foundation'
import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { Flex } from '@ui/components/legacy'
import { useState } from 'react'

import { InvoicesTable } from '@/features/invoices'
import { useQuery } from '@/lib/connectrpc'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

type Props = {
  subscriptionId: string
}

export const SubscriptionInvoicesCard = ({ subscriptionId }: Props) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 5,
  })

  const invoicesQuery = useQuery(listInvoices, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    subscriptionId,
    sortBy: ListInvoicesRequest_SortBy.DATE_DESC,
  })

  return invoicesQuery.isLoading ? (
    <Flex direction="column" gap={spaces.space9} fullHeight>
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </Flex>
  ) : (
    <InvoicesTable
      data={invoicesQuery.data?.invoices || []}
      totalCount={invoicesQuery.data?.paginationMeta?.totalItems || 0}
      pagination={pagination}
      setPagination={setPagination}
      isLoading={invoicesQuery.isLoading}
      linkPrefix="../../billing/invoices/"
    />
  )
}
