import { CouponsHeader } from '@/features/productCatalog/coupons/CouponsHeader'
import { CouponsTable } from '@/features/productCatalog/coupons/CouponsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useState } from 'react'
import { Outlet } from 'react-router-dom'

export const CouponsPage: FunctionComponent = () => {
  const [search] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const debouncedSearch = useDebounceValue(search, 200)

  const couponsQuery = useQuery(listCoupons, {
    pagination: {
      limit: pagination.pageSize,
      offset: pagination.pageIndex,
    },
    search: debouncedSearch,
    filter: ListCouponRequest_CouponFilter.ALL,
  })

  return (
    <div className="h-full w-full flex flex-row gap-16">
      <div className="flex flex-col gap-5 h-full w-full">
        <CouponsHeader />
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
