import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useState } from 'react'
import { Outlet } from 'react-router-dom'

import { AddonsHeader } from '@/features/productCatalog/addons/AddonsHeader'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddonsPage: FunctionComponent = () => {
  const [search] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const debouncedSearch = useDebounceValue(search, 200)

  const addonsQuery = useQuery(listAddOns, {
    search: debouncedSearch,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
  })

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
