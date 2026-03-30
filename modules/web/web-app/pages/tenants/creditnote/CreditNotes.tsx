import { Fragment, useCallback, useEffect, useState } from 'react'

import { CreditNotesHeader, CreditNotesTable } from '@/features/creditNotes'
import { CreditNotesSearch } from '@/features/creditNotes/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listCreditNotes } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'

import type { PaginationState, SortingState } from '@tanstack/react-table'

export const CreditNotes = () => {
  const [search, setSearch] = useState<CreditNotesSearch>({})

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const [sorting, setSorting] = useState<SortingState>([{ id: 'created_at', desc: true }])

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

  const creditNotesQuery = useQuery(
    listCreditNotes,
    {
      orderBy: sortingStateToOrderBy(sorting),
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
  const count = creditNotesQuery.data?.paginationMeta?.totalItems ?? 0
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
          sorting={sorting}
          onSortingChange={handleSortingChange}
        />
      </div>
    </Fragment>
  )
}
