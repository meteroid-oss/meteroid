import { Button, Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { FileSignature, Plus } from 'lucide-react'
import { useState } from 'react'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { QuotesTable } from '@/features/quotes/QuotesTable'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listQuotes } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

type Props = {
  customer: Customer
  /** Creates one for this customer; the empty state offers it when set. */
  onCreate?: () => void
}

export const QuotesCard = ({ customer, onCreate }: Props) => {
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

  const total = Number(quotesQuery.data?.paginationMeta?.totalItems ?? 0)
  if (!quotesQuery.isLoading && total === 0) {
    return (
      <SectionEmptyState
        framed={false}
        icon={<FileSignature />}
        title="No quotes yet"
        action={
          onCreate && (
            <Button size="sm" variant="secondary" hasIcon onClick={onCreate}>
              <Plus size={14} /> New quote
            </Button>
          )
        }
      >
        Quotes you prepare for this customer appear here, from draft to accepted.
      </SectionEmptyState>
    )
  }

  return quotesQuery.isLoading ? (
    <div className="flex flex-col gap-4 p-4">
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
