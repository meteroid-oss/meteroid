import { PaginationState, SortingState } from '@tanstack/react-table'
import { Fragment, FunctionComponent, useCallback, useEffect, useState } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { ProductMetricsPageHeader } from '@/features/productCatalog/metrics/ProductMetricsPageHeader'
import { BillableMetricTable } from '@/features/productCatalog/metrics/ProductMetricsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

export const ProductMetrics: FunctionComponent = () => {
  const navigate = useNavigate()
  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [statusFilter, setStatusFilter] = useState<'all' | 'active' | 'archived'>('active')
  const [sorting, setSorting] = useState<SortingState>([])

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [statusFilter, debouncedSearch])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
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
    <Fragment>
      <div className="flex flex-col gap-8">
        <ProductMetricsPageHeader
          setEditPanelVisible={() => navigate('add-metric')}
          isLoading={isLoading}
          refetch={refetch}
          statusFilter={statusFilter}
          onStatusFilterChange={setStatusFilter}
          totalCount={totalCount}
          search={search}
          setSearch={setSearch}
        />
        <BillableMetricTable
          data={data}
          totalCount={totalCount}
          pagination={pagination}
          setPagination={setPagination}
          sorting={sorting}
          onSortingChange={handleSortingChange}
          isLoading={isLoading}
        />
      </div>
      <Outlet />
    </Fragment>
  )
}
