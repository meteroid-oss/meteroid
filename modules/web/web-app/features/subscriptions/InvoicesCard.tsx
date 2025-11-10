import { spaces } from '@md/foundation'
import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { Flex } from '@ui/components/legacy'
import { useEffect, useState } from 'react'

import { InvoicesTable } from '@/features/invoices'
import { useQuery } from '@/lib/connectrpc'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

type Props = {
  subscriptionId: string
  onRefetchChange?: (refetch: () => void, isFetching: boolean) => void
}

export const SubscriptionInvoicesCard = ({ subscriptionId, onRefetchChange }: Props) => {
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

  // Notify parent of refetch function and loading state
  useEffect(() => {
    if (onRefetchChange) {
      onRefetchChange(() => invoicesQuery.refetch(), invoicesQuery.isFetching)
    }
  }, [onRefetchChange, invoicesQuery.refetch, invoicesQuery.isFetching])

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
      isLoading={invoicesQuery.isFetching}
      linkPrefix="../../billing/invoices/"
    />
  )
}
