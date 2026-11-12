import { ColumnDef, OnChangeFn, PaginationState, Row, SortingState } from '@tanstack/react-table'
import { format } from 'date-fns'
import { ReactNode, useMemo } from 'react'

import { ColumnState, StandardTable } from '@/components/table/StandardTable'
import {
  useCustomPropertyColumns,
  withCustomPropertyColumns,
} from '@/features/customproperties/useCustomPropertyColumns'
import { SubscriptionStatusBadge } from '@/features/subscriptions/SubscriptionStatusBadge'
import { formatUsageFilter } from '@/features/subscriptions/UsageFilterFields'
import { useBasePath } from '@/hooks/useBasePath'
import { useCurrency } from '@/hooks/useCurrency'
import { usePrefetch } from '@/hooks/usePrefetch'
import { mapDateFromGrpcv2 } from '@/lib/mapping'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { FunctionComponent } from 'react'

interface SubscriptionsTableProps {
  data: Subscription[]
  tableKey?: string
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
  isLoading?: boolean
  hideCustomer?: boolean
  hidePlan?: boolean
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  chart?: ReactNode
  /** Column visibility controlled by an active saved view. */
  columnState?: ColumnState
}

export const SubscriptionsTable: FunctionComponent<SubscriptionsTableProps> = ({
  data,
  tableKey,
  pagination,
  setPagination,
  totalCount,
  isLoading,
  hideCustomer = false,
  hidePlan = false,
  sorting,
  onSortingChange,
  chart,
  columnState,
}) => {
  const { formatAmount } = useCurrency()
  const basePath = useBasePath()
  const prefetch = usePrefetch()

  const { columns: customPropertyColumns, hiddenIds: customPropertyHiddenIds } =
    useCustomPropertyColumns<Subscription>(CustomPropertyEntityType.SUBSCRIPTION)

  const baseColumns = useMemo<ColumnDef<Subscription>[]>(
    () =>
      [
        {
          id: 'customer_name',
          header: 'Customer',
          accessorKey: 'customerName',
        },
        {
          id: 'plan_name',
          header: 'Plan',
          accessorKey: 'planName',
          cell: ({ row }: { row: Row<Subscription> }) => (
            <span>
              {row.original.planName}
              {row.original.usageFilter && (
                <span className="ml-2 text-xs text-muted-foreground">
                  {formatUsageFilter(row.original.usageFilter)}
                </span>
              )}
            </span>
          ),
        },

        {
          header: 'Version',
          accessorKey: 'version',
          enableSorting: false,
        },
        {
          id: 'mrr_cents',
          header: 'MRR',
          accessorFn: (cell: Subscription) =>
            cell.mrrCents > 0 ? formatAmount(cell.mrrCents) : null,
        },
        {
          id: 'billing_start_date',
          header: 'Start date',
          accessorFn: (cell: Subscription) =>
            cell.billingStartDate
              ? format(mapDateFromGrpcv2(cell.billingStartDate), 'dd/MM/yyyy')
              : '',
        },
        {
          id: 'end_date',
          header: 'End date',
          enableSorting: true,
          cell: ({ row }: { row: Row<Subscription> }) =>
            row.original.endDate
              ? format(mapDateFromGrpcv2(row.original.endDate), 'dd/MM/yyyy')
              : null,
        },

        {
          id: 'status',
          header: 'Status',
          enableSorting: true,
          cell: ({ row }: { row: Row<Subscription> }) => (
            <SubscriptionStatusBadge status={row.original.status} />
          ),
        },
        {
          header: 'Currency',
          accessorKey: 'currency',
          enableSorting: false,
        },
      ]
        .filter(col => !hideCustomer || col.header !== 'Customer')
        .filter(col => !hidePlan || col.header !== 'Plan'),

    [hideCustomer, hidePlan, formatAmount]
  )

  // Custom-property columns are opt-in via the view menu, which only exists when the table
  // has a `tableKey` (persisted visibility). Skip them on embedded, keyless lists.
  const columns = useMemo(
    () => (tableKey ? withCustomPropertyColumns(baseColumns, customPropertyColumns) : baseColumns),
    [baseColumns, customPropertyColumns, tableKey]
  )

  return (
    <StandardTable
      tableKey={tableKey}
      columnState={columnState}
      columns={columns}
      data={data}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
      defaultHidden={tableKey ? customPropertyHiddenIds : undefined}
      rowLink={row => `${basePath}/subscriptions/${row.original.id}`}
      onRowIntent={row => prefetch(getSubscriptionDetails, { subscriptionId: row.original.id })}
      chart={chart}
    />
  )
}
