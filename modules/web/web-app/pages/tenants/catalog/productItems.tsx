import { PlusIcon } from '@md/icons'
import { Button } from '@md/ui'
import { FunctionComponent, useCallback, useState } from 'react'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { BaseFilter } from '@/features/TablePage'
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
      count={totalCount}
      actions={
        <Gate perm="catalog:manage">
          <Button variant="primary" size="sm" hasIcon onClick={() => setIsCreating(true)}>
            <PlusIcon className="w-4 h-4" /> New product
          </Button>
        </Gate>
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search ?? ''}
            onChange={handleSearch}
            placeholder="Search products"
            className="max-w-[220px]"
          />
          <BaseFilter
            entries={[
              { label: 'Active', value: 'active' },
              { label: 'Archived', value: 'archived' },
            ]}
            emptyLabel="Active"
            selected={[statusFilter]}
            onSelectionChange={(value, checked) => {
              if (!checked) return
              setStatusFilter(value as 'active' | 'archived')
              setPagination(prev => ({ ...prev, pageIndex: 0 }))
            }}
          />
        </>
      }
      toolbarTrailing={<RefreshButton isLoading={isLoading} onClick={refetch} />}
    >
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
