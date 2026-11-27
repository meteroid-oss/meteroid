import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useState } from 'react'
import { useMatch, useNavigate, useParams, useResolvedPath } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, PanelHead, RefreshButton } from '@/components/layouts'
import { AddonCreateForm } from '@/features/productCatalog/addons/AddonCreateForm'
import { AddonDetailPanel } from '@/features/productCatalog/addons/AddonDetailPanel'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddonsPage: FunctionComponent = () => {
  const navigate = useNavigate()
  // The add-on open beside the list, or "New add-on" in its place.
  const { addonId } = useParams<{ addonId?: string }>()
  const creating = useMatch(useResolvedPath('add-addon').pathname) !== null
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

  // No add-on at all gets the empty state; a search that finds none keeps the table.
  const isEmpty =
    !debouncedSearch &&
    pagination.pageIndex === 0 &&
    addonsQuery.isSuccess &&
    (addonsQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  return (
    <ListPageLayout
      title="Add-ons"
      panel={
        creating ? (
          <div className="divide-y divide-border">
            <PanelHead
              title={<h2 className="text-base font-semibold">New add-on</h2>}
              onClose={() => navigate('.')}
            />
            <div className="px-4 py-4">
              <AddonCreateForm
                submitLabel="Create add-on"
                onCancel={() => navigate('.')}
                onCreated={addOn => navigate(addOn.id)}
              />
            </div>
          </div>
        ) : (
          addonId && (
            <AddonDetailPanel key={addonId} addonId={addonId} onClose={() => navigate('.')} />
          )
        )
      }
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-addon')}>
            <AddIcon /> New add-on
          </Button>
        </Gate>
      }
      toolbar={
        <ListToolbarSearch
          value={search ?? ''}
          onChange={setSearch}
          placeholder="Search add-ons"
        />
      }
      toolbarTrailing={
        <RefreshButton isLoading={addonsQuery.isLoading} onClick={() => addonsQuery.refetch()} />
      }
    >
      {isEmpty ? (
        <EmptyState
          title="No add-ons yet"
          description="Add-ons are extras a customer can attach to their subscription."
          scene="addons"
          actions={
            <Gate perm="catalog:manage">
              <Button size="sm" variant="default" onClick={() => navigate('add-addon')}>
                New add-on
              </Button>
            </Gate>
          }
        />
      ) : (
        <AddonsTable
          addonsQuery={addonsQuery}
          pagination={pagination}
          setPagination={setPagination}
          sorting={sorting}
          onSortingChange={handleSortingChange}
          // Narrower beside "New add-on" too, no row marked.
          selectedId={creating ? '' : addonId}
        />
      )}
    </ListPageLayout>
  )
}
