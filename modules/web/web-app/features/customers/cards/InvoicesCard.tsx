import { spaces } from '@md/foundation'
import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { Flex } from '@ui/components/legacy'
import { useState } from 'react'

import { InvoicesTable } from '@/features/invoices'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

type Props = {
  customer: Customer
}

export const InvoicesCard = ({ customer }: Props) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const invoicesQuery = useQuery(listInvoices, {
    pagination: {
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
    },
    customerId: customer.id,
    orderBy: ListInvoicesRequest_SortBy.DATE_DESC,
  })

  return invoicesQuery.isLoading ? (
    <Flex direction="column" gap={spaces.space9} fullHeight>
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </Flex>
  ) : (
    <InvoicesTable
      data={invoicesQuery.data?.invoices || []}
      totalCount={invoicesQuery.data?.paginationMeta?.total || 0}
      pagination={pagination}
      setPagination={setPagination}
      isLoading={invoicesQuery.isLoading}
      linkPrefix="../../invoices/"
    />
  )
}
