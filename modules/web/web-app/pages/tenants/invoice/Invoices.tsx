import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { Fragment, useState } from 'react'

import { InvoicesHeader, InvoicesTable } from '@/features/invoices'
import { InvoicesSearch } from '@/features/invoices/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Invoices = () => {
  const [, setEditPanelVisible] = useState(false)
  const [search, setSearch] = useState<InvoicesSearch>({})

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const invoicesQuery = useQuery(
    listInvoices,
    {
      sortBy: ListInvoicesRequest_SortBy.DATE_DESC,
      search: debouncedSearch.text || '',
      status: debouncedSearch.status,
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
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
          totalCount={invoicesQuery.data?.paginationMeta?.totalItems || 0}
          pagination={pagination}
          setPagination={setPagination}
          isLoading={isLoading}
        />
      </Flex>
    </Fragment>
  )
}
