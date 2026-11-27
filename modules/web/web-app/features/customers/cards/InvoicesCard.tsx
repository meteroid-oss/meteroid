import { Button, Skeleton } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ReceiptText, Plus } from 'lucide-react'
import { useState } from 'react'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { InvoicesTable } from '@/features/invoices'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'

type Props = {
  customer: Customer
  /** Creates one for this customer; the empty state offers it when set. */
  onCreate?: () => void
}

export const InvoicesCard = ({ customer, onCreate }: Props) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 5,
  })

  const invoicesQuery = useQuery(listInvoices, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    customerId: customer.id,
  })

  const total = Number(invoicesQuery.data?.paginationMeta?.totalItems ?? 0)
  if (!invoicesQuery.isLoading && total === 0) {
    return (
      <SectionEmptyState
        framed={false}
        icon={<ReceiptText />}
        title="No invoices yet"
        action={
          onCreate && (
            <Button size="sm" variant="secondary" hasIcon onClick={onCreate}>
              <Plus size={14} /> New invoice
            </Button>
          )
        }
      >
        Invoices for this customer appear here once a subscription bills or you create one.
      </SectionEmptyState>
    )
  }

  return invoicesQuery.isLoading ? (
    <div className="flex flex-col gap-4 p-4">
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
    />
  )
}
