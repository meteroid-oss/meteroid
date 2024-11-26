import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { AddOn } from '@/rpc/api/addons/v1/models_pb'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'

import { ListAddOnResponse } from '@/rpc/api/addons/v1/addons_pb'
import { UseQueryResult } from '@tanstack/react-query'
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
        cell: ({ row }) => <span>{row.original.name}</span>,
        enableSorting: false,
      },

      {
        header: 'Fee type',
        cell: ({ row }) => <span>{row.original.fee?.feeType.case}</span>,
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
      totalCount={addonsQuery.data?.addOns?.length ?? 0}
      isLoading={addonsQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
