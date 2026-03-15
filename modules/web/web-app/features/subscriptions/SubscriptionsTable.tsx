import { ColumnDef, OnChangeFn, PaginationState, Row } from '@tanstack/react-table'
import { format } from 'date-fns'
import { useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { SubscriptionStatusBadge } from '@/features/subscriptions/SubscriptionStatusBadge'
import { useBasePath } from '@/hooks/useBasePath'
import { useCurrency } from '@/hooks/useCurrency'
import { mapDateFromGrpcv2 } from '@/lib/mapping'
import { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

import type { FunctionComponent } from 'react'

interface SubscriptionsTableProps {
  data: Subscription[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  hideCustomer?: boolean
  hidePlan?: boolean
}

export const SubscriptionsTable: FunctionComponent<SubscriptionsTableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  hideCustomer = false,
  hidePlan = false,
}) => {
  const { formatAmount } = useCurrency()
  const basePath = useBasePath()

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
          header: 'MRR',
          accessorKey: 'mrrCents',
          accessorFn: (cell: Subscription) =>
            cell.mrrCents > 0 ? formatAmount(cell.mrrCents) : null,
        },
        {
          header: 'Start date',
          accessorFn: (cell: Subscription) =>
            cell.billingStartDate
              ? format(mapDateFromGrpcv2(cell.billingStartDate), 'dd/MM/yyyy')
              : '',
          enableSorting: false,
        },
        {
          header: 'End date',
          cell: ({ row }: { row: Row<Subscription> }) =>
            row.original.endDate
              ? format(mapDateFromGrpcv2(row.original.endDate), 'dd/MM/yyyy')
              : null,
          enableSorting: false,
        },

        {
          header: 'Status',
          cell: ({ row }: { row: Row<Subscription> }) => <SubscriptionStatusBadge status={row.original.status} />,
        },
        {
          header: 'Currency',
          accessorKey: 'currency',
          enableSorting: false,
        },
      ]
        .filter(col => !hideCustomer || col.header !== 'Customer')
        .filter(col => !hidePlan || col.header !== 'Plan'),

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
      rowLink={row => `${basePath}/subscriptions/${row.original.id}`}
    />
  )
}

