import { PaginationState, SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useEffect, useState } from 'react'

import { AddonsHeader } from '@/features/productCatalog/addons/AddonsHeader'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddonsPage: FunctionComponent = () => {
  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [sorting, setSorting] = useState<SortingState>([])

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

  const addonsQuery = useQuery(listAddOns, {
    search: debouncedSearch || undefined,
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
    orderBy: sortingStateToOrderBy(sorting),
  })

  return (
    <>
      <AddonsHeader count={addonsQuery.data?.paginationMeta?.totalItems} search={search} setSearch={setSearch} />
      <AddonsTable
        addonsQuery={addonsQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
    </>
  )
}
