import { Button, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { FunctionComponent, useCallback, useState } from 'react'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { ProductCreatePanel } from '@/features/productCatalog/items/ProductCreatePanel'
import { ProductDetailPanel } from '@/features/productCatalog/items/ProductDetailPanel'
import { ProductsTable } from '@/features/productCatalog/items/ProductItemsTable'
import { useListPagination } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { searchProducts } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import type { SortingState } from '@tanstack/react-table'

export const Products: FunctionComponent = () => {
  const [selectedProductId, setSelectedProductId] = useState<string | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [search, setSearch] = useState<string | undefined>(undefined)
  const [statusFilter, setStatusFilter] = useState<'active' | 'archived'>('active')
  const [pagination, setPagination] = useListPagination()
  const [sorting, setSorting] = useState<SortingState>([])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const productsQuery = useQuery(searchProducts, {
    query: search || undefined,
    archived: statusFilter === 'archived',
    pagination: { perPage: pagination.pageSize, page: pagination.pageIndex },
    orderBy: sortingStateToOrderBy(sorting),
  })
  const data = productsQuery.data?.products ?? []
  const isLoading = productsQuery.isLoading
  const totalCount = productsQuery.data?.paginationMeta?.totalItems ?? 0
  // No product at all gets the empty state: the Active tab empty, and no archived product either.
  const activeTabEmpty =
    statusFilter === 'active' &&
    !search &&
    pagination.pageIndex === 0 &&
    productsQuery.isSuccess &&
    totalCount === 0
  const archivedProductsQuery = useQuery(
    searchProducts,
    { archived: true, pagination: { perPage: 1, page: 0 } },
    { enabled: activeTabEmpty }
  )
  const isEmpty =
    activeTabEmpty &&
    archivedProductsQuery.isSuccess &&
    (archivedProductsQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  const refetch = () => {
    productsQuery.refetch()
  }

  const handleSearch = (value: string | undefined) => {
    setSearch(value)
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }

  return (
    <ListPageLayout
      title="Products"
      // The products' state, as tabs in the top bar like the other catalogue lists.
      tabs={
        <Tabs
          value={statusFilter}
          onValueChange={value => {
            setStatusFilter(value as 'active' | 'archived')
            setPagination(prev => ({ ...prev, pageIndex: 0 }))
          }}
        >
          <TabsList variant="ghost">
            <TabsTrigger value="active">Active</TabsTrigger>
            <TabsTrigger value="archived">Archived</TabsTrigger>
          </TabsList>
        </Tabs>
      }
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => setIsCreating(true)}>
            <AddIcon /> New product
          </Button>
        </Gate>
      }
      toolbar={
        <ListToolbarSearch value={search ?? ''} onChange={handleSearch} placeholder="Search products" />
      }
      toolbarTrailing={<RefreshButton isLoading={isLoading} onClick={refetch} />}
    >
      {isEmpty ? (
        <EmptyState
          title="No products yet"
          description="Products are what you sell. Add one, then price it in a plan."
          scene="products"
          actions={
            <Gate perm="catalog:manage">
              <Button size="sm" variant="default" onClick={() => setIsCreating(true)}>
                New product
              </Button>
            </Gate>
          }
        />
      ) : (
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
      )}
      <ProductDetailPanel
        productId={selectedProductId}
        onClose={() => setSelectedProductId(null)}
      />
      <Gate perm="catalog:manage">
        <ProductCreatePanel
          visible={isCreating}
          onClose={() => setIsCreating(false)}
          onCreated={setSelectedProductId}
        />
      </Gate>
    </ListPageLayout>
  )
}
