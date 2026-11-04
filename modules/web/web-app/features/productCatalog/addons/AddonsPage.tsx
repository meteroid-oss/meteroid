import { PlusIcon } from '@md/icons'
import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddonsPage: FunctionComponent = () => {
  const navigate = useNavigate()
  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useListPagination()
  const [sorting, setSorting] = useState<SortingState>([])

  useResetPageOnChange(setPagination, [debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
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
    <ListPageLayout
      title="Add-ons"
      count={addonsQuery.data?.paginationMeta?.totalItems}
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-addon')}>
            <PlusIcon className="w-4 h-4" /> New add-on
          </Button>
        </Gate>
      }
      toolbar={
        <ListToolbarSearch
          value={search ?? ''}
          onChange={setSearch}
          placeholder="Search add-ons"
          className="max-w-[220px]"
        />
      }
      toolbarTrailing={
        <RefreshButton isLoading={addonsQuery.isLoading} onClick={() => addonsQuery.refetch()} />
      }
    >
      <AddonsTable
        addonsQuery={addonsQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
    </ListPageLayout>
  )
}
