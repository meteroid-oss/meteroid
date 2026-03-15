import { PaginationState } from '@tanstack/react-table'
import { Tabs, TabsList, TabsTrigger } from '@ui/components'
import { FunctionComponent, useState } from 'react'
import { Outlet } from 'react-router-dom'

import { CouponsHeader } from '@/features/productCatalog/coupons/CouponsHeader'
import { CouponsTable } from '@/features/productCatalog/coupons/CouponsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'

export const CouponsPage: FunctionComponent = () => {
  const [search] = useQueryState<string | undefined>('q', undefined)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const debouncedSearch = useDebounceValue(search, 200)
  const [statusFilter, setStatusFilter] = useQueryState<string>('filter', 'active')

  const filter =
    statusFilter === 'all'
      ? ListCouponRequest_CouponFilter.ALL
      : statusFilter === 'inactive'
        ? ListCouponRequest_CouponFilter.INACTIVE
        : statusFilter === 'archived'
          ? ListCouponRequest_CouponFilter.ARCHIVED
          : ListCouponRequest_CouponFilter.ACTIVE

  const couponsQuery = useQuery(listCoupons, {
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
    search: debouncedSearch,
    filter: filter,
  })

  return (
    <div className="h-full w-full flex flex-row gap-16">
      <div className="flex flex-col gap-5 h-full w-full">
        <CouponsHeader
          count={couponsQuery.data?.paginationMeta?.totalItems}
          isLoading={couponsQuery.isLoading}
          refetch={() => couponsQuery.refetch()}
        >
          <Tabs value={statusFilter} onValueChange={setStatusFilter}>
            <TabsList>
              <TabsTrigger value="all">All</TabsTrigger>
              <TabsTrigger value="active">Active</TabsTrigger>
              <TabsTrigger value="inactive">Inactive</TabsTrigger>
              <TabsTrigger value="archived">Archived</TabsTrigger>
            </TabsList>
          </Tabs>
        </CouponsHeader>
        <CouponsTable
          couponsQuery={couponsQuery}
          pagination={pagination}
          setPagination={setPagination}
        />
      </div>
      <Outlet />
    </div>
  )
}
