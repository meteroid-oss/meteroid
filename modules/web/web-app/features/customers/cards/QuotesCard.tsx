import { Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { useState } from 'react'

import { QuotesTable } from '@/features/quotes/QuotesTable'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listQuotes } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

type Props = {
  customer: Customer
}

export const QuotesCard = ({ customer }: Props) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 5,
  })

  const quotesQuery = useQuery(listQuotes, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    customerId: customer.id,
  })

  return quotesQuery.isLoading ? (
    <div className="flex flex-col gap-8 h-full">
      <Skeleton height={16} width={50} />
      <Skeleton height={44} />
    </div>
  ) : (
    <QuotesTable
      data={quotesQuery.data?.quotes || []}
      totalCount={Number(quotesQuery.data?.paginationMeta?.totalItems ?? 0)}
      pagination={pagination}
      setPagination={setPagination}
      isLoading={quotesQuery.isLoading}
      hideCustomer
    />
  )
}
