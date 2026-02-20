import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import { StandardTable } from '@/components/table/StandardTable'
import { ListAddOnResponse } from '@/rpc/api/addons/v1/addons_pb'
import { AddOn } from '@/rpc/api/addons/v1/models_pb'

import type { FunctionComponent } from 'react'

interface AddonsTableProps {
  addonsQuery: UseQueryResult<ListAddOnResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
}
export const AddonsTable: FunctionComponent<AddonsTableProps> = ({
  addonsQuery,
  pagination,
  setPagination,
}) => {
  const navigate = useNavigate()

  const columns = useMemo<ColumnDef<AddOn>[]>(
    () => [
      {
        header: 'Name',
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
        enableSorting: false,
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
        header: 'Self-service',
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {row.original.selfServiceable ? 'Yes' : 'No'}
          </span>
        ),
        enableSorting: false,
      },

      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => <LocalId localId={row.original.localId} className="max-w-16" />,
      },
    ],
    [navigate]
  )

  return (
    <StandardTable
      columns={columns}
      data={addonsQuery.data?.addOns ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={addonsQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={addonsQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
