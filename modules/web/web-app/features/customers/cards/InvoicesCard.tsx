import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { useState } from 'react'

import { InvoicesTable } from '@/features/invoices'
import { useBasePath } from '@/hooks/useBasePath'
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
    pageSize: 5,
  })

  const basePath = useBasePath()

  const invoicesQuery = useQuery(listInvoices, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    customerId: customer.id,
    sortBy: ListInvoicesRequest_SortBy.DATE_DESC,
  })

  return invoicesQuery.isLoading ? (
    <div className="flex flex-col gap-8 h-full">
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </div>
  ) : (
    <InvoicesTable
      data={invoicesQuery.data?.invoices || []}
      totalCount={invoicesQuery.data?.paginationMeta?.totalItems || 0}
      pagination={pagination}
      setPagination={setPagination}
      isLoading={invoicesQuery.isLoading}
      linkPrefix={`${basePath}/invoices/`}
    />
  )
}
