import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { EyeIcon } from 'lucide-react'
import { FC, useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { ProductMeta } from '@/rpc/api/products/v1/models_pb'

import type { OnChangeFn } from '@tanstack/react-table'

interface ProductsTableProps {
  data: ProductMeta[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  onProductClick?: (product: ProductMeta) => void
}
export const ProductsTable: FC<ProductsTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  onProductClick,
}) => {
  const columns = useMemo<ColumnDef<ProductMeta>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
      },
      {
        header: 'Local ID',
        accessorKey: 'localId',
        cell: ({ getValue }) => (
          <span className="font-mono text-xs text-muted-foreground">{getValue<string>()}</span>
        ),
      },
      {
        accessorKey: 'id',
        header: '',
        className: 'w-2',
        cell: ({ row }) => (
          <button
            onClick={e => {
              e.stopPropagation()
              onProductClick?.(row.original)
            }}
            className="cursor-pointer text-muted-foreground hover:text-foreground"
          >
            <EyeIcon size={16} />
          </button>
        ),
      },
    ],
    [onProductClick]
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
    />
  )
}
