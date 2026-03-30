import { ColumnDef, PaginationState, SortingState } from '@tanstack/react-table'
import { FC, useMemo } from 'react'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { feeTypeLabel } from '@/lib/mapping/prices'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { ProductMeta } from '@/rpc/api/products/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

import type { OnChangeFn } from '@tanstack/react-table'

interface ProductsTableProps {
  data: ProductMeta[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  onProductClick?: (product: ProductMeta) => void
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
}
export const ProductsTable: FC<ProductsTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  onProductClick,
  sorting,
  onSortingChange,
}) => {
  const columns = useMemo<ColumnDef<ProductMeta>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        enableSorting: true,
        cell: ({ row }) => (
          <button
            className="text-left cursor-pointer"
            onClick={() => onProductClick?.(row.original)}
          >
            <span className="font-medium">{row.original.name}</span>
            {row.original.description && (
              <span className="block text-xs text-muted-foreground truncate max-w-xs">
                {row.original.description}
              </span>
            )}
          </button>
        ),
      },
      {
        header: 'Fee Type',
        cell: ({ row }) => {
          const ft = row.original.feeType
          if (ft === undefined) return null
          return <span className="text-sm">{feeTypeLabel(ft as FeeType)}</span>
        },
        enableSorting: false,
      },
      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => <LocalId localId={row.original.localId} className="max-w-16" />,
        enableSorting: false,
      },
      {
        id: 'created_at',
        header: 'Created',
        enableSorting: true,
        cell: ({ row }) => {
          const ts = row.original.createdAt
          if (!ts) return null
          return (
            <span className="text-sm text-muted-foreground">
              {parseAndFormatDate(ts)}
            </span>
          )
        },
      },
    ],
    [onProductClick]
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
    />
  )
}
