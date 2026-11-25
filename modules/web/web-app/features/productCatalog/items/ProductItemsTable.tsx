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
import { StandardTable } from '@/components/table/StandardTable'
import { countsWords, priceWords } from '@/features/plans/canvas/billingWords'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import { protoFeeTypeToComponentFeeType } from '@/features/pricing/formData'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { useProductArchive } from '@/features/productCatalog/items/useProductArchive'
import { env } from '@/lib/env'
import { ProductMeta } from '@/rpc/api/products/v1/models_pb'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { OnChangeFn } from '@tanstack/react-table'

const contextOf = (product: ProductMeta): ChargeContext => ({
  product,
  metric: product.metricName ? { name: product.metricName } : undefined,
})

/** What it charges, in the plan editor's words: "Flat fee", "Usage of Tokens", "Per seat". */
const chargesWords = (product: ProductMeta) => {
  const ctx = contextOf(product)
  const [price] = product.latestPrices
  if (price) return countsWords(price, ctx)
  if (product.feeType === undefined) return ''
  const kind = protoFeeTypeToComponentFeeType(product.feeType)
  if ((kind === 'usage' || kind === 'capacity') && product.metricName) {
    return `Usage of ${product.metricName}`
  }
  return feeTypeToHuman(kind)
}

/** Its latest price in each currency: "$99 / month · €89 / month". */
const pricesWords = (product: ProductMeta) =>
  product.latestPrices.map(price => priceWords(price, contextOf(product))).join(' · ')

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`

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
            className="flex min-w-0 cursor-pointer items-center gap-3 text-left"
            onClick={() => onProductClick?.(row.original)}
          >
            <ProductIcon icon={row.original.icon} color={row.original.color} />
            <span className="min-w-0">
              <span className="flex items-baseline gap-2">
                <span className="font-medium">{row.original.name}</span>
                {/* Faded in, its room kept: the columns don't move on hover. */}
                <span className="font-mono text-[11px] text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100">
                  {row.original.localId}
                </span>
              </span>
              {row.original.description && (
                <span className="block max-w-xs truncate text-xs text-muted-foreground">
                  {row.original.description}
                </span>
              )}
            </span>
          </button>
        ),
      },
      {
        header: 'Charges',
        enableSorting: false,
        cell: ({ row }) => {
          const prices = pricesWords(row.original)
          return (
            <span className="block min-w-0">
              <span className="block">{chargesWords(row.original)}</span>
              {prices && (
                <span className="block max-w-sm truncate text-xs text-muted-foreground" title={prices}>
                  {prices}
                </span>
              )}
            </span>
          )
        },
      },
      {
        header: 'Plans',
        enableSorting: false,
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {row.original.planCount
              ? `In ${plural(row.original.planCount, 'plan')}`
              : 'In no plan'}
          </span>
        ),
      },
      ...(env.entitlementsEnabled
        ? [
            {
              header: 'Features',
              enableSorting: false,
              cell: ({ row }) => (
                <span className="text-muted-foreground">
                  {row.original.featureCount ? plural(row.original.featureCount, 'feature') : '—'}
                </span>
              ),
            } satisfies ColumnDef<ProductMeta>,
          ]
        : []),
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
      rowClassName={() => 'group'}
    />
  )
}
