import { Button, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useState } from 'react'
import { Outlet, useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { BillableMetricTable } from '@/features/productCatalog/metrics/ProductMetricsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

export const ProductMetrics: FunctionComponent = () => {
  const navigate = useNavigate()
  const isExpress = useIsExpressOrganization()
  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useListPagination()
  const [statusFilter, setStatusFilter] = useState<'active' | 'archived'>('active')
  const [sorting, setSorting] = useState<SortingState>([])

  useResetPageOnChange(setPagination, [statusFilter, debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const archivedParam = statusFilter === 'archived'

  const productMetricsQuery = useQuery(listBillableMetrics, {
    archived: archivedParam,
    search: debouncedSearch || undefined,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
    orderBy: sortingStateToOrderBy(sorting),
  })

  const totalCount = productMetricsQuery?.data?.paginationMeta?.totalItems ?? 0
  const isLoading = productMetricsQuery.isLoading
  const data = productMetricsQuery.data?.billableMetrics ?? []
  // No metric at all gets the empty state: the Active tab empty, and no archived metric either.
  const activeTabEmpty =
    statusFilter === 'active' &&
    !debouncedSearch &&
    pagination.pageIndex === 0 &&
    productMetricsQuery.isSuccess &&
    totalCount === 0
  const archivedMetricsQuery = useQuery(
    listBillableMetrics,
    { archived: true, pagination: { page: 0, perPage: 1 } },
    { enabled: activeTabEmpty }
  )
  const isEmpty =
    activeTabEmpty &&
    archivedMetricsQuery.isSuccess &&
    (archivedMetricsQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  const refetch = () => {
    productMetricsQuery.refetch()
  }

  return (
    <ListPageLayout
      title="Metrics"
      // The metrics' state, as tabs in the top bar like the other catalogue lists.
      tabs={
        <Tabs
          value={statusFilter}
          onValueChange={value => setStatusFilter(value as 'active' | 'archived')}
        >
          <TabsList variant="ghost">
            <TabsTrigger value="active">Active</TabsTrigger>
            <TabsTrigger value="archived">Archived</TabsTrigger>
          </TabsList>
        </Tabs>
      }
      actions={
        isExpress ? undefined : (
          <Gate perm="catalog:manage">
            <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-metric')}>
              <AddIcon /> New metric
            </Button>
          </Gate>
        )
      }
      toolbar={<ListToolbarSearch value={search} onChange={setSearch} placeholder="Search metrics" />}
      toolbarTrailing={<RefreshButton isLoading={isLoading} onClick={refetch} />}
    >
      {isEmpty ? (
        <EmptyState
          title="No metrics yet"
          description="Metrics count the usage you bill for, like API calls or storage."
          scene="metrics"
          actions={
            isExpress ? undefined : (
              <Gate perm="catalog:manage">
                <Button size="sm" variant="default" onClick={() => navigate('add-metric')}>
                  New metric
                </Button>
              </Gate>
            )
          }
        />
      ) : (
        <BillableMetricTable
          data={data}
          totalCount={totalCount}
          pagination={pagination}
          setPagination={setPagination}
          sorting={sorting}
          onSortingChange={handleSortingChange}
          isLoading={isLoading}
        />
      )}
      <Outlet />
    </ListPageLayout>
  )
}
