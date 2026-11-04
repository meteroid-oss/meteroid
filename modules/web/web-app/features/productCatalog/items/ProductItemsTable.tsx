import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { ColumnDef, PaginationState, SortingState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, MoreVerticalIcon } from 'lucide-react'
import { FC, useMemo } from 'react'

import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { useProductArchive } from '@/features/productCatalog/items/useProductArchive'
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
  const { archive, unarchive } = useProductArchive()

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
          return<>{feeTypeLabel(ft as FeeType)}</>
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
            <span className=" text-muted-foreground">
              {parseAndFormatDate(ts)}
            </span>
          )
        },
      },
      {
        header: 'Status',
        enableSorting: false,
        cell: ({ row }) =>
          row.original.archivedAt ? (
            <Badge variant="secondary">Archived</Badge>
          ) : (
            <Badge variant="success">Active</Badge>
          ),
      },
      {
        id: 'actions',
        header: '',
        maxSize: 0.1,
        enableSorting: false,
        cell: ({ row }) => (
          <Gate perm="catalog:manage">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm">
                  <MoreVerticalIcon size={16} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {row.original.archivedAt ? (
                  <DropdownMenuItem onClick={() => unarchive(row.original.id)}>
                    <ArchiveRestoreIcon size={16} className="mr-2" />
                    Unarchive
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem onClick={() => archive(row.original.id)}>
                    <ArchiveIcon size={16} className="mr-2" />
                    Archive
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </Gate>
        ),
      },
    ],
    [onProductClick, archive, unarchive]
  )

  return (
    <StandardTable
      tableKey="product_items"
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
