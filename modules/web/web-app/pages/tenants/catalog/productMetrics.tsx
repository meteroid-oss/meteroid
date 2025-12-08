import { PaginationState } from '@tanstack/react-table'
import { Fragment, FunctionComponent, useState } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { ProductMetricsPageHeader } from '@/features/productCatalog/metrics/ProductMetricsPageHeader'
import { BillableMetricTable } from '@/features/productCatalog/metrics/ProductMetricsTable'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

export const ProductMetrics: FunctionComponent = () => {
  const navigate = useNavigate()
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [statusFilter, setStatusFilter] = useState<'all' | 'active' | 'archived'>('active')

  const archivedParam = statusFilter === 'all' ? undefined : statusFilter === 'archived'

  const productMetricsQuery = useQuery(listBillableMetrics, {
    archived: archivedParam,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
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
        />
        <BillableMetricTable
          data={data}
          totalCount={totalCount}
          pagination={pagination}
          setPagination={setPagination}
        />
      </div>
      <Outlet />
    </Fragment>
  )
}
