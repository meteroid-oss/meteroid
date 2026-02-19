import { Fragment, FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { ProductDetailPanel } from '@/features/productCatalog/items/ProductDetailPanel'
import { ProductsHeader } from '@/features/productCatalog/items/ProductItemsHeader'
import { ProductsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useQuery } from '@/lib/connectrpc'
import { listProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import type { PaginationState } from '@tanstack/react-table'

export const Products: FunctionComponent = () => {
  const [selectedProductId, setSelectedProductId] = useState<string | null>(null)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const productsQuery = useQuery(listProducts, {
    pagination: { perPage: pagination.pageSize, page: pagination.pageIndex },
  })
  const data = productsQuery.data?.products ?? []
  const isLoading = productsQuery.isLoading
  const totalCount = productsQuery.data?.paginationMeta?.totalItems ?? 0

  const refetch = () => {
    productsQuery.refetch()
  }

  return (
    <Fragment>
      <ProductsHeader
        heading="Product Items"
        setEditPanelVisible={() => false}
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
        onProductClick={product => setSelectedProductId(product.id)}
      />
      <ProductDetailPanel
        productId={selectedProductId}
        onClose={() => setSelectedProductId(null)}
      />
    </Fragment>
  )
}
