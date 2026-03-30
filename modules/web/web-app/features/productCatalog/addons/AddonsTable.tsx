import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'


import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { feeTypeToHuman, priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
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
}
export const AddonsTable: FunctionComponent<AddonsTableProps> = ({
  addonsQuery,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
}) => {
  const navigate = useNavigate()

  const columns = useMemo<ColumnDef<AddOn>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        enableSorting: true,
        cell: ({ row }) => (
          <div>
            <span className="font-medium">{row.original.name}</span>
            {row.original.description && (
              <span className="block text-xs text-muted-foreground truncate max-w-xs">
                {row.original.description}
              </span>
            )}
          </div>
        ),
      },

      {
        header: 'Fee Type',
        cell: ({ row }) => (
          <span className="text-sm">
            {feeTypeToHuman(feeTypeEnumToComponentFeeType(row.original.feeType))}
          </span>
        ),
        enableSorting: false,
      },

      {
        header: 'Price',
        cell: ({ row }) => {
          const feeType = feeTypeEnumToComponentFeeType(row.original.feeType)
          const badges = priceSummaryBadges(feeType, row.original.price, row.original.price?.currency)
          return (
            <span className="text-sm text-muted-foreground">
              {badges.join(' / ')}
            </span>
          )
        },
        enableSorting: false,
      },

      {
        header: 'Self-service',
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {row.original.selfServiceable ? 'Yes' : 'No'}
          </span>
        ),
        enableSorting: false,
      },

      {
        id: 'created_at',
        header: 'Created',
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
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
    ],
    [navigate]
  )

  return (
    <StandardTable
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
    />
  )
}
