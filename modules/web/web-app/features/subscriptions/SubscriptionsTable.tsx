import { Badge } from '@md/ui'
import { ColumnDef, OnChangeFn, PaginationState, Row, SortingState } from '@tanstack/react-table'
import { format } from 'date-fns'
import { ReactNode, useMemo } from 'react'

import { ColumnState, StandardTable } from '@/components/table/StandardTable'
import {
  useCustomPropertyColumns,
  withCustomPropertyColumns,
} from '@/features/customproperties/useCustomPropertyColumns'
import { useResourceChips } from '@/features/resources/hooks'
import { SubscriptionStatusBadge } from '@/features/subscriptions/SubscriptionStatusBadge'
import { TrialStatusBadge } from '@/features/subscriptions/trial/TrialStatusBadge'
import { onFileLabel, trialPaymentMethod } from '@/features/subscriptions/trial/trialList'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
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
  /** The trial's end, payment method and status, from each row's trial summary. */
  trialColumns?: boolean
}

const trialPaymentMethodCell = (subscription: Subscription) => {
  switch (trialPaymentMethod(subscription)) {
    case 'invoice':
      return 'Invoice'
    case 'on_file':
      return onFileLabel(subscription)
    case 'none':
      return (
        <Badge variant="warning" size="sm">
          None
        </Badge>
      )
    default:
      return '—'
  }
}

const TRIAL_COLUMNS: ColumnDef<Subscription>[] = [
  {
    id: 'trial_ends_on',
    header: 'Trial ends',
    enableSorting: false,
    cell: ({ row }: { row: Row<Subscription> }) =>
      row.original.trial ? formatShortDay(row.original.trial.endsOn) : '—',
  },
  {
    id: 'trial_payment_method',
    header: 'Payment method',
    enableSorting: false,
    cell: ({ row }: { row: Row<Subscription> }) => trialPaymentMethodCell(row.original),
  },
  {
    id: 'trial_status',
    header: 'Trial',
    enableSorting: false,
    cell: ({ row }: { row: Row<Subscription> }) =>
      row.original.trial ? <TrialStatusBadge status={row.original.trial.status} /> : '—',
  },
]

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
  trialColumns = false,
}) => {
  const { formatAmount } = useCurrency()
  const basePath = useBasePath()
  const prefetch = usePrefetch()
  const chips = useResourceChips()

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
              {row.original.resourceId && (
                <span className="ml-2 text-xs text-muted-foreground">
                  {chips.get(row.original.resourceId) ?? 'a resource'}
                </span>
              )}
            </span>
          ),
        },
        ...(trialColumns ? TRIAL_COLUMNS : []),
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

    [hideCustomer, hidePlan, formatAmount, chips, trialColumns]
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
