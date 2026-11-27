import { SortingState } from '@tanstack/react-table'
import { Button } from '@ui/components'
import { useState } from 'react'
import { useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { FeaturesTable } from '@/features/entitlements/features/FeaturesTable'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { FeatureActionDialog, PendingFeatureAction } from './statusActions'

const ALL_STATUSES: FeatureStatus[] = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED, FeatureStatus.ARCHIVED]
const DEFAULT_STATUSES: FeatureStatus[] = [FeatureStatus.ACTIVE, FeatureStatus.DISABLED]

function statusLabel(s: FeatureStatus): string {
  switch (s) {
    case FeatureStatus.ACTIVE:
      return 'Active'
    case FeatureStatus.DISABLED:
      return 'Disabled'
    case FeatureStatus.ARCHIVED:
      return 'Archived'
    default:
      return String(s)
  }
}


export const FeaturesPage = () => {
  const navigate = useNavigate()

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const debouncedSearch = useDebounceValue(search, 400)
  const [statusSelection, setStatusSelection] = useState<Set<FeatureStatus>>(
    () => new Set(DEFAULT_STATUSES)
  )
  const [productFilter, setProductFilter] = useQueryState<string | undefined>('product', undefined)
  const [pagination, setPagination] = useListPagination()
  const [sorting, setSorting] = useState<SortingState>([])
  const [pendingAction, setPendingAction] = useState<PendingFeatureAction | null>(null)

  useResetPageOnChange(setPagination, [debouncedSearch, statusSelection, productFilter])

  const productsQuery = useQuery(listProducts, { pagination: { page: 0, perPage: 200 } })
  const products = productsQuery.data?.products ?? []

  const isDefaultStatuses =
    statusSelection.size === DEFAULT_STATUSES.length &&
    DEFAULT_STATUSES.every(status => statusSelection.has(status))

  const query = useQuery(listFeatures, {
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
    statuses: Array.from(statusSelection),
    search: debouncedSearch || undefined,
    productId: productFilter || undefined,
  })


  // No feature at all gets the empty state. The default view hides archived features, so when it
  // is empty one feature of any status is looked for first.
  const isNarrowed = !!debouncedSearch || !!productFilter || !isDefaultStatuses
  const defaultViewEmpty =
    !isNarrowed &&
    pagination.pageIndex === 0 &&
    query.isSuccess &&
    (query.data?.paginationMeta?.totalItems ?? 0) === 0
  const anyFeatureQuery = useQuery(
    listFeatures,
    { pagination: { page: 0, perPage: 1 }, statuses: ALL_STATUSES },
    { enabled: defaultViewEmpty }
  )
  const isEmpty =
    defaultViewEmpty &&
    anyFeatureQuery.isSuccess &&
    (anyFeatureQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  return (
    <ListPageLayout
      title="Features"
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('create')}>
            <AddIcon /> New feature
          </Button>
        </Gate>
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search ?? ''}
            onChange={setSearch}
            placeholder="Search features"
          />
          <FilterBar
            local={[
              {
                field: optionField(
                  'status',
                  'Status',
                  ALL_STATUSES.map(status => ({ value: String(status), label: statusLabel(status) }))
                ),
                // The baseline (all but archived) is no filter: a chip only for another choice.
                values: isDefaultStatuses ? [] : Array.from(statusSelection).map(String),
                set: values =>
                  setStatusSelection(
                    new Set(
                      values.length > 0
                        ? values.map(value => Number(value) as FeatureStatus)
                        : DEFAULT_STATUSES
                    )
                  ),
              },
              {
                field: optionField(
                  'product',
                  'Product',
                  products.map(product => ({ value: product.id, label: product.name })),
                  { multi: false }
                ),
                values: productFilter ? [productFilter] : [],
                set: ([value]) => setProductFilter(value),
              },
            ]}
          />
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={query.isLoading} onClick={() => query.refetch()} />}
    >
      {isEmpty ? (
        <EmptyState
          title="No features yet"
          description="Features decide what each plan gives access to, from simple switches to usage limits."
          scene="features"
          actions={
            <Gate perm="catalog:manage">
              <Button size="sm" variant="default" onClick={() => navigate('create')}>
                New feature
              </Button>
            </Gate>
          }
        />
      ) : (
        <FeaturesTable
          query={query}
          pagination={pagination}
          setPagination={setPagination}
          sorting={sorting}
          onSortingChange={setSorting}
          onAction={(feature, target) => setPendingAction({ feature, target })}
        />
      )}

      <FeatureActionDialog pending={pendingAction} onClose={() => setPendingAction(null)} />
    </ListPageLayout>
  )
}
