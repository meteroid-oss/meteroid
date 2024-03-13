import { disableQuery } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { FunctionComponent, useState } from 'react'

import { ProductEditPanel } from '@/features/productCatalog/items/ProductEditPanel'
import { ProductItemsHeader } from '@/features/productCatalog/items/ProductItemsHeader'
import { ProductItemsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useQuery } from '@/lib/connectrpc'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { useTypedParams } from '@/utils/params'

import type { PaginationState } from '@tanstack/react-table'

export const Addons: FunctionComponent = () => {
  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()

  const productsQuery = useQuery(
    listProducts,
    familyExternalId
      ? {
          familyExternalId,
        }
      : disableQuery
  )
  const data = productsQuery.data?.products ?? []
  const isLoading = productsQuery.isLoading
  const totalCount = productsQuery.data?.paginationMeta?.total ?? 0

  const refetch = () => {
    productsQuery.refetch()
  }

  return (
    <Flex direction="column" gap={spaces.space9}>
      <ProductItemsHeader
        heading="Addons"
        setEditPanelVisible={setEditPanelVisible}
        isLoading={isLoading}
        refetch={refetch}
      />
      <ProductItemsTable
        data={data}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={isLoading}
      />
      <ProductEditPanel visible={editPanelVisible} closePanel={() => setEditPanelVisible(false)} />
    </Flex>
  )
}
