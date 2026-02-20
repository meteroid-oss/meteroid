import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useState } from 'react'

import { AddonsHeader } from '@/features/productCatalog/addons/AddonsHeader'
import { AddonsTable } from '@/features/productCatalog/addons/AddonsTable'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddonsPage: FunctionComponent = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const addonsQuery = useQuery(listAddOns, {
    pagination: {
      perPage: pagination.pageSize,
      page: pagination.pageIndex,
    },
  })

  return (
    <>
      <AddonsHeader count={addonsQuery.data?.paginationMeta?.totalItems} />
      <AddonsTable
        addonsQuery={addonsQuery}
        pagination={pagination}
        setPagination={setPagination}
      />
    </>
  )
}
