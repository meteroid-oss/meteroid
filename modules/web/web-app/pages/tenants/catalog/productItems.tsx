import { disableQuery } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { Fragment, FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { ProductEditPanel } from '@/features/productCatalog/items/ProductEditPanel'
import { ProductsHeader } from '@/features/productCatalog/items/ProductItemsHeader'
import { ProductsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useQuery } from '@/lib/connectrpc'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { useTypedParams } from '@/utils/params'

import type { PaginationState } from '@tanstack/react-table'

export const Products: FunctionComponent = () => {
  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

  const productsQuery = useQuery(listProducts, familyLocalId ? { familyLocalId } : disableQuery)
  const data = productsQuery.data?.products ?? []
  const isLoading = productsQuery.isLoading
  const totalCount = productsQuery.data?.products?.length ?? 0 // no server pagination ? TODO

  const refetch = () => {
    productsQuery.refetch()
  }

  return (
    <Fragment>
      <Flex direction="column" gap={spaces.space9}>
        <ProductsHeader
          heading="Product Items"
          setEditPanelVisible={setEditPanelVisible}
          isLoading={isLoading}
          refetch={refetch}
          setSearch={() => {
            toast.error('Search not implemented')
          }}
        />
        <ProductsTable
          data={data}
          pagination={pagination}
          setPagination={setPagination}
          totalCount={totalCount}
          isLoading={isLoading}
        />
      </Flex>
      <ProductEditPanel visible={editPanelVisible} closePanel={() => setEditPanelVisible(false)} />
    </Fragment>
  )
}
