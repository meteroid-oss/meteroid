import { ColumnDef, OnChangeFn, PaginationState, Row } from '@tanstack/react-table'
import { Pill } from '@ui/components'
import { format } from 'date-fns'
import { useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { mapDateFromGrpc } from '@/lib/mapping'
import { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

import type { FunctionComponent } from 'react'

interface SubscriptionsTableProps {
  data: Subscription[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  hideCustomer?: boolean
}

export const SubscriptionsTable: FunctionComponent<SubscriptionsTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  hideCustomer = false,
}) => {
  const columns = useMemo<ColumnDef<Subscription>[]>(
    () =>
      [
        {
          header: 'Customer',
          accessorKey: 'customerName',
        },
        {
          header: 'Plan',
          accessorKey: 'planName',
        },
        {
          header: 'Version',
          accessorKey: 'version',
          enableSorting: false,
        },
        {
          header: 'Start date',
          accessorFn: (cell: Subscription) =>
            cell.billingStartDate
              ? format(mapDateFromGrpc(cell.billingStartDate), 'dd/MM/yyyy')
              : '',
          enableSorting: false,
        },
        {
          header: 'End date',
          cell: ({ row }: { row: Row<Subscription> }) =>
            row.original.billingEndDate ? (
              format(mapDateFromGrpc(row.original.billingEndDate), 'dd/MM/yyyy')
            ) : (
              <Pill color="success">Active</Pill>
            ),
          enableSorting: false,
        },
        {
          header: 'Currency',
          accessorKey: 'currency',
          enableSorting: false,
        },
      ].filter(col => !hideCustomer || col.header !== 'Customer'),
    [hideCustomer]
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
