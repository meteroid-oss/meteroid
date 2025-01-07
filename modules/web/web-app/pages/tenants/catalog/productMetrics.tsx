import { spaces } from '@md/foundation'
import { PaginationState } from '@tanstack/react-table'
import { Flex } from '@ui/components/legacy'
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

  // TODO pagination (manual ?)
  const productMetricsQuery = useQuery(listBillableMetrics, {})

  const totalCount = productMetricsQuery?.data?.billableMetrics?.length ?? 0
  const isLoading = productMetricsQuery.isLoading
  const data = productMetricsQuery.data?.billableMetrics ?? []

  const refetch = () => {
    productMetricsQuery.refetch()
  }

  return (
    <Fragment>
      <Flex direction="column" gap={spaces.space9}>
        <ProductMetricsPageHeader
          setEditPanelVisible={() => navigate('add-metric')}
          isLoading={isLoading}
          refetch={refetch}
        />
        <BillableMetricTable
          data={data}
          totalCount={totalCount}
          pagination={pagination}
          setPagination={setPagination}
        />
      </Flex>
      <Outlet />
    </Fragment>
  )
}
