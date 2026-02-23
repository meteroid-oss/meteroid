import { Fragment, FunctionComponent, useState } from 'react'

import { ProductDetailPanel } from '@/features/productCatalog/items/ProductDetailPanel'
import { ProductsHeader } from '@/features/productCatalog/items/ProductItemsHeader'
import { ProductsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useQuery } from '@/lib/connectrpc'
import { searchProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import type { PaginationState } from '@tanstack/react-table'

export const Products: FunctionComponent = () => {
  const [selectedProductId, setSelectedProductId] = useState<string | null>(null)
  const [search, setSearch] = useState<string | undefined>(undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const productsQuery = useQuery(searchProducts, {
    query: search || undefined,
    pagination: { perPage: pagination.pageSize, page: pagination.pageIndex },
  })
  const data = productsQuery.data?.products ?? []
  const isLoading = productsQuery.isLoading
  const totalCount = productsQuery.data?.paginationMeta?.totalItems ?? 0

  const refetch = () => {
    productsQuery.refetch()
  }

  const handleSearch = (value: string | undefined) => {
    setSearch(value)
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }

  return (
    <Fragment>
      <ProductsHeader
        heading="Product Items"
        setEditPanelVisible={() => false}
        isLoading={isLoading}
        refetch={refetch}
        setSearch={handleSearch}
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
