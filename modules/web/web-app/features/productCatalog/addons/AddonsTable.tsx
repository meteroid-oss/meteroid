import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router'


import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { countsWords } from '@/features/plans/canvas/billingWords'
import { priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { protoFeeTypeToComponentFeeType } from '@/features/pricing/formData'
import { addOnPrices } from '@/features/productCatalog/addons/addonWords'
import { ProductIcon } from '@/features/productCatalog/items/ProductIcon'
import { ListAddOnResponse } from '@/rpc/api/addons/v1/addons_pb'
import { AddOn } from '@/rpc/api/addons/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

import type { FunctionComponent } from 'react'

interface AddonsTableProps {
  addonsQuery: UseQueryResult<ListAddOnResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  /** The add-on open beside the list: its row is marked, the list narrower. */
  selectedId?: string
}
export const AddonsTable: FunctionComponent<AddonsTableProps> = ({
  addonsQuery,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  selectedId,
}) => {
  const navigate = useNavigate()
  const creditUnits = useCreditUnits()
  const compact = selectedId !== undefined

  const columns = useMemo<ColumnDef<AddOn>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        enableSorting: true,
        cell: ({ row }) => (
          <div className="flex items-center gap-3">
            <ProductIcon icon={row.original.productIcon} color={row.original.productColor} />
            <div className="min-w-0">
              <span className="font-medium">{row.original.name}</span>
              {row.original.description && (
                <span className="block text-xs text-muted-foreground truncate max-w-xs">
                  {row.original.description}
                </span>
              )}
            </div>
          </div>
        ),
      },

      {
        header: 'Charges for',
        cell: ({ row }) => (
          <span>
            {countsWords(addOnPrices(row.original)[0], {
              product: row.original.feeStructure
                ? { feeStructure: row.original.feeStructure }
                : undefined,
            })}
          </span>
        ),
        enableSorting: false,
      },

      {
        header: 'Price',
        cell: ({ row }) => {
          const feeType = protoFeeTypeToComponentFeeType(row.original.feeType)
          const badges = priceSummaryBadges(
            feeType,
            row.original.price,
            row.original.price?.currency,
            creditUnits
          )
          const others = new Set(
            row.original.prices.map(p => p.currency).filter(c => c !== row.original.price?.currency)
          ).size
          return (
            <span className="  text-muted-foreground">
              {badges.join(' / ')}
              {others > 0 && (
                <span className="ml-1 text-xs">
                  +{others} {others === 1 ? 'currency' : 'currencies'}
                </span>
              )}
            </span>
          )
        },
        enableSorting: false,
      },

      {
        header: 'Self-service',
        cell: ({ row }) => (
          <span className="  text-muted-foreground">
            {row.original.selfServiceable ? 'Yes' : 'No'}
          </span>
        ),
        enableSorting: false,
      },

      ...((compact
        ? []
        : [
            {
              id: 'created_at',
              header: 'Created',
              cell: ({ row }) => (
                <span className="  text-muted-foreground">
                  {parseAndFormatDate(row.original.createdAt)}
                </span>
              ),
            },
            {
              header: 'API Handle',
              id: 'localId',
              cell: ({ row }) => <LocalId localId={row.original.id} className="max-w-16" />,
              enableSorting: false,
            },
          ]) as ColumnDef<AddOn>[]),
    ],
    [navigate, creditUnits, compact]
  )

  return (
    <StandardTable
      tableKey="addons"
      columns={columns}
      data={addonsQuery.data?.addOns ?? []}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={addonsQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={addonsQuery.isLoading}
      rowLink={row => `${row.original.id}`}
      rowClassName={row => (row.original.id === selectedId ? 'bg-accent/50' : '')}
    />
  )
}
