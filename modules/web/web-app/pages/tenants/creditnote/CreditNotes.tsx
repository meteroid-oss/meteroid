import { Fragment, useState } from 'react'

import { CreditNotesHeader, CreditNotesTable } from '@/features/creditNotes'
import { CreditNotesSearch } from '@/features/creditNotes/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listCreditNotes } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'
import { ListCreditNotesRequest_SortBy } from '@/rpc/api/creditnotes/v1/creditnotes_pb'

import type { PaginationState } from '@tanstack/react-table'

export const CreditNotes = () => {
  const [search, setSearch] = useState<CreditNotesSearch>({})

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const creditNotesQuery = useQuery(
    listCreditNotes,
    {
      sortBy: ListCreditNotesRequest_SortBy.DATE_DESC,
      search: debouncedSearch.text || undefined,
      status: debouncedSearch.status,
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
    },
    {}
  )

  const data = creditNotesQuery.data?.creditNotes ?? []
  const count = data.length
  const isLoading = creditNotesQuery.isLoading

  const refetch = () => {
    creditNotesQuery.refetch()
  }

  return (
    <Fragment>
      <div className="flex flex-col gap-8">
        <CreditNotesHeader
          count={count}
          isLoading={isLoading}
          refetch={refetch}
          setSearch={setSearch}
          search={search}
        />
        <CreditNotesTable
          data={data}
          totalCount={creditNotesQuery.data?.paginationMeta?.totalItems || 0}
          pagination={pagination}
          setPagination={setPagination}
          isLoading={isLoading}
        />
      </div>
    </Fragment>
  )
}
