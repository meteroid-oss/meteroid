import { PlusIcon } from '@md/icons'
import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useState } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { BaseFilter } from '@/features/TablePage'
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
  const [statusFilter, setStatusFilter] = useState<'all' | 'active' | 'archived'>('active')
  const [sorting, setSorting] = useState<SortingState>([])

  useResetPageOnChange(setPagination, [statusFilter, debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const archivedParam = statusFilter === 'all' ? undefined : statusFilter === 'archived'

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

  const refetch = () => {
    productMetricsQuery.refetch()
  }

  return (
    <ListPageLayout
      title="Metrics"
      count={totalCount}
      actions={
        isExpress ? undefined : (
          <Gate perm="catalog:manage">
            <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-metric')}>
              <PlusIcon className="w-4 h-4" /> New metric
            </Button>
          </Gate>
        )
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search}
            onChange={setSearch}
            placeholder="Search metrics"
            className="max-w-[220px]"
          />
          <BaseFilter
            entries={[
              { label: 'Active', value: 'active' },
              { label: 'Archived', value: 'archived' },
            ]}
            emptyLabel="All"
            selected={statusFilter !== 'all' ? [statusFilter] : []}
            onSelectionChange={(value, checked) =>
              setStatusFilter(checked ? (value as 'active' | 'archived') : 'all')
            }
          />
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={isLoading} onClick={refetch} />}
    >
      <BillableMetricTable
        data={data}
        totalCount={totalCount}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        isLoading={isLoading}
      />
      <Outlet />
    </ListPageLayout>
  )
}
