import { Fragment, useCallback, useEffect, useState } from 'react'

import { InvoicesHeader, InvoicesTable } from '@/features/invoices'
import { InvoicesSearch } from '@/features/invoices/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'

import type { PaginationState, SortingState } from '@tanstack/react-table'

export const Invoices = () => {
  const [, setEditPanelVisible] = useState(false)
  const [search, setSearch] = useState<InvoicesSearch>({})
  const [sorting, setSorting] = useState<SortingState>([{ id: 'invoice_date', desc: true }])

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

  const invoicesQuery = useQuery(
    listInvoices,
    {
      orderBy: sortingStateToOrderBy(sorting),
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
  const count = invoicesQuery.data?.paginationMeta?.totalItems ?? 0
  const isLoading = invoicesQuery.isLoading

  const refetch = () => {
    invoicesQuery.refetch()
  }

  return (
    <Fragment>
      <div className="flex flex-col gap-8">
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
          sorting={sorting}
          onSortingChange={handleSortingChange}
        />
      </div>
    </Fragment>
  )
}
