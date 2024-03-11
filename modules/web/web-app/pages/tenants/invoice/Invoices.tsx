import { spaces } from '@md/foundation'
import { Flex } from '@ui/components'
import { Fragment, useState } from 'react'

import { TenantPageLayout } from '@/components/layouts'
import { InvoicesHeader, InvoicesTable } from '@/features/invoices'
import { InvoicesSearch } from '@/features/invoices/types'
import useDebounce from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Invoices = () => {
  const [, setEditPanelVisible] = useState(false)
  const [search, setSearch] = useState<InvoicesSearch>({})

  const debouncedSearch = useDebounce(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const invoicesQuery = useQuery(
    listInvoices,
    {
      orderBy: ListInvoicesRequest_SortBy.DATE_DESC,
      search: debouncedSearch.text || '',
      status: debouncedSearch.status,
      pagination: {
        limit: pagination.pageSize,
        offset: pagination.pageIndex * pagination.pageSize,
      },
    },
    {}
  )

  const data = invoicesQuery.data?.invoices ?? []
  const count = data.length
  const isLoading = invoicesQuery.isLoading

  const refetch = () => {
    invoicesQuery.refetch()
  }

  return (
    <Fragment>
      <Flex direction="column" gap={spaces.space9}>
        <InvoicesHeader
          count={count}
          setEditPanelVisible={setEditPanelVisible}
          isLoading={isLoading}
          refetch={refetch}
          setSearch={setSearch}
          search={search}
        />
        <InvoicesTable
          data={data}
          totalCount={count}
          pagination={pagination}
          setPagination={setPagination}
          isLoading={isLoading}
        />
      </Flex>
    </Fragment>
  )
}
