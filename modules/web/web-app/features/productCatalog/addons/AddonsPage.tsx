import { AddonsHeader } from '@/features/productCatalog/addons/AddonsHeader'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useMemo, useState } from 'react'
import { Outlet } from 'react-router-dom'

export const AddonsPage: FunctionComponent = () => {
  const productFamiliesQuery = useQuery(listProductFamilies)

  const [search] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const productFamilyData = useMemo(
    () =>
      productFamiliesQuery.data?.productFamilies.map(pf => ({
        label: pf.name,
        value: pf.localId,
      })) ?? [],
    [productFamiliesQuery.data]
  )

  const debouncedSearch = useDebounceValue(search, 200)

  const addonsQuery = useQuery(listAddOns, {})

  return (
    <>
      <AddonsHeader />
      <AddonsTable
        addonsQuery={addonsQuery}
        pagination={pagination}
        setPagination={setPagination}
      />
      <Outlet />
    </>
  )
}
