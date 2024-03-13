import { disableQuery } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { PaginationState } from '@tanstack/react-table'
import { Flex } from '@ui2/components/legacy'
import { Fragment, FunctionComponent, useState } from 'react'

import { ProductMetricsEditPanel } from '@/features/productCatalog/metrics/ProductMetricsEditPanel'
import { ProductMetricsPageHeader } from '@/features/productCatalog/metrics/ProductMetricsPageHeader'
import { BillableMetricTable } from '@/features/productCatalog/metrics/ProductMetricsTable'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { useTypedParams } from '@/utils/params'
import { Outlet, useNavigate } from 'react-router-dom'

export const ProductMetrics: FunctionComponent = () => {
  const navigate = useNavigate()
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()
  // TODO pagination (manual ?)
  const productMetricsQuery = useQuery(
    listBillableMetrics,
    familyExternalId ? { familyExternalId } : disableQuery
  )

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
