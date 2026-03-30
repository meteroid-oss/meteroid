import { Fragment, FunctionComponent, useCallback, useState } from 'react'

import { ProductDetailPanel } from '@/features/productCatalog/items/ProductDetailPanel'
import { ProductsHeader } from '@/features/productCatalog/items/ProductItemsHeader'
import { ProductsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { searchProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import type { PaginationState, SortingState } from '@tanstack/react-table'

export const Products: FunctionComponent = () => {
  const [selectedProductId, setSelectedProductId] = useState<string | null>(null)
  const [search, setSearch] = useState<string | undefined>(undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [sorting, setSorting] = useState<SortingState>([])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

  const productsQuery = useQuery(searchProducts, {
    query: search || undefined,
    pagination: { perPage: pagination.pageSize, page: pagination.pageIndex },
    orderBy: sortingStateToOrderBy(sorting),
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
        count={totalCount}
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
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
      <ProductDetailPanel
        productId={selectedProductId}
        onClose={() => setSelectedProductId(null)}
      />
    </Fragment>
  )
}
