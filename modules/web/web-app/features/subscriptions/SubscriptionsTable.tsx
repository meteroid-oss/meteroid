import { ColumnDef, OnChangeFn, PaginationState, Row } from '@tanstack/react-table'
import { Badge } from '@ui/components'
import { format } from 'date-fns'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { useCurrency } from '@/hooks/useCurrency'
import { mapDateFromGrpcv2 } from '@/lib/mapping'
import { Subscription, SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'

import { useBasePath } from '@/hooks/useBasePath'
import type { FunctionComponent, ReactNode } from 'react'

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
          cell: ({ row }: { row: Row<Subscription> }) => (
            <Link to={`${basePath}/customers/${row.original.customerId}`}>
              {row.original.customerName}
            </Link>
          ),
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
            row.original.billingEndDate
              ? format(mapDateFromGrpcv2(row.original.billingEndDate), 'dd/MM/yyyy')
              : null,
          enableSorting: false,
        },

        {
          header: 'Status',
          cell: ({ row }: { row: Row<Subscription> }) => formatStatus(row.original.status),
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

function formatStatus(status: SubscriptionStatus): ReactNode {
  switch (status) {
    case SubscriptionStatus.ACTIVE:
      return <Badge variant="success">Active</Badge>
    case SubscriptionStatus.CANCELED:
      return <Badge variant="secondary">Canceled</Badge>
    case SubscriptionStatus.ENDED:
      return <Badge variant="secondary">Ended</Badge>
    case SubscriptionStatus.PENDING:
      return <Badge variant="warning">Pending</Badge>
    case SubscriptionStatus.TRIALING:
      return <Badge variant="outline">Trial</Badge>
    default:
      return 'Unknown'
  }
}
