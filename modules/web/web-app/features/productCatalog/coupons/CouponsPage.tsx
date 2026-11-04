import { PlusIcon } from '@md/icons'
import { SortingState } from '@tanstack/react-table'
import { Button, Tabs, TabsList, TabsTrigger } from '@ui/components'
import { FunctionComponent, useCallback, useState } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { CouponsTable } from '@/features/productCatalog/coupons/CouponsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'

export const CouponsPage: FunctionComponent = () => {
  const navigate = useNavigate()
  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)

  const [pagination, setPagination] = useListPagination()

  const debouncedSearch = useDebounceValue(search, 200)
  const [statusFilter, setStatusFilter] = useQueryState<string>('filter', 'active')
  const [sorting, setSorting] = useState<SortingState>([])

  useResetPageOnChange(setPagination, [debouncedSearch, statusFilter])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

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
    orderBy: sortingStateToOrderBy(sorting),
  })

  return (
    <ListPageLayout
      title="Coupons"
      count={couponsQuery.data?.paginationMeta?.totalItems}
      tabs={
        <Tabs value={statusFilter} onValueChange={setStatusFilter}>
          <TabsList variant="ghost">
            <TabsTrigger value="all">All</TabsTrigger>
            <TabsTrigger value="active">Active</TabsTrigger>
            <TabsTrigger value="inactive">Inactive</TabsTrigger>
            <TabsTrigger value="archived">Archived</TabsTrigger>
          </TabsList>
        </Tabs>
      }
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-coupon')}>
            <PlusIcon className="w-4 h-4" /> New coupon
          </Button>
        </Gate>
      }
      toolbar={
        <ListToolbarSearch
          value={search ?? ''}
          onChange={setSearch}
          placeholder="Search coupons"
          className="max-w-[220px]"
        />
      }
      toolbarTrailing={
        <RefreshButton isLoading={couponsQuery.isLoading} onClick={() => couponsQuery.refetch()} />
      }
    >
      <CouponsTable
        couponsQuery={couponsQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
      <Outlet />
    </ListPageLayout>
  )
}
