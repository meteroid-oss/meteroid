import { spaces } from '@md/foundation'
import { PaginationState } from '@tanstack/react-table'
import { Flex, Skeleton } from '@ui/components'
import { ComponentProps, useState } from 'react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { InvoicesTable } from '@/features/invoices'
import { useQuery } from '@/lib/connectrpc'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

type Props = Pick<ComponentProps<typeof PageSection>, 'className'> & {
  customer: Customer
}

export const InvoicesCard = ({ customer, className }: Props) => {
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

  return (
    <PageSection
      className={className}
      header={{
        title: 'Invoices',
      }}
    >
      {invoicesQuery.isLoading ? (
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
      )}
    </PageSection>
  )
}
