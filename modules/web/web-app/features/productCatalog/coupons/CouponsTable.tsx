import { cn } from '@md/ui'
import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { useMemo } from 'react'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { getCountryName } from '@/features/settings/utils'
import { ListCouponResponse } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

import { couponAppliesTo, couponDiscountLine, couponStatus, couponWhoLine } from './couponWords'

import type { CouponStatus } from './couponWords'
import type { FunctionComponent } from 'react'

interface CouponsTableProps {
  couponsQuery: UseQueryResult<ListCouponResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  /** The coupon open beside the list: its row is marked, the list narrower. */
  selectedLocalId?: string
  /** A catalog offer's name; a coupon giving its own offer has none. */
  offerName?: (offerId: string) => string | undefined
}

const STATUS_STYLE: Record<CouponStatus, string> = {
  Active: 'bg-success/15 text-success',
  Scheduled: 'bg-muted text-muted-foreground',
  Disabled: 'bg-muted text-muted-foreground',
  Archived: 'bg-muted text-muted-foreground',
  Exhausted: 'bg-warning/15 text-warning',
  Expired: 'bg-warning/15 text-warning',
}

export const CouponStatusChip = ({ status }: { status: CouponStatus }) => (
  <span className={cn('rounded px-1.5 py-0.5 text-xs', STATUS_STYLE[status])}>{status}</span>
)

export const CouponsTable: FunctionComponent<CouponsTableProps> = ({
  couponsQuery,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  selectedLocalId,
  offerName,
}) => {
  const isCompact = selectedLocalId !== undefined

  const columns = useMemo<ColumnDef<Coupon>[]>(
    () => [
      {
        id: 'code',
        header: 'Coupon',
        enableSorting: true,
        cell: ({ row }) => (
          <div className="min-w-0">
            <span className="font-mono font-medium">{row.original.code}</span>
            {row.original.description && (
              <span className="block max-w-xs truncate text-xs text-muted-foreground">
                {row.original.description}
              </span>
            )}
          </div>
        ),
      },
      {
        header: 'Discount',
        enableSorting: false,
        cell: ({ row }) => <span>{couponDiscountLine(row.original)}</span>,
      },
      {
        header: 'Offer',
        enableSorting: false,
        cell: ({ row }) => {
          const name = row.original.offerId ? offerName?.(row.original.offerId) : undefined
          return name ? <span>{name}</span> : <span className="text-muted-foreground">—</span>
        },
      },
      {
        // Sorts on the expiry the line ends with.
        id: 'expires_at',
        header: 'Who can redeem it',
        enableSorting: true,
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {couponWhoLine(row.original, code => getCountryName(code))}
          </span>
        ),
      },
      ...((isCompact
        ? []
        : [
            {
              header: 'Applies to',
              enableSorting: false,
              cell: ({ row }) => (
                <span className="text-muted-foreground">{couponAppliesTo(row.original)}</span>
              ),
            },
          ]) as ColumnDef<Coupon>[]),
      {
        header: 'Redemptions',
        enableSorting: false,
        cell: ({ row }) => (
          <span className="tabular-nums">
            {row.original.redemptionCount}
            <span className="text-muted-foreground"> / {row.original.redemptionLimit ?? '∞'}</span>
          </span>
        ),
      },
      {
        header: 'Status',
        enableSorting: false,
        cell: ({ row }) => <CouponStatusChip status={couponStatus(row.original)} />,
      },
      ...((isCompact
        ? []
        : [
            {
              id: 'created_at',
              header: 'Created',
              enableSorting: true,
              cell: ({ row }) => (
                <span className="text-muted-foreground">
                  {parseAndFormatDate(row.original.createdAt)}
                </span>
              ),
            },
            {
              header: 'API Handle',
              id: 'localId',
              cell: ({ row }) => <LocalId localId={row.original.localId} className="max-w-16" />,
              enableSorting: false,
            },
          ]) as ColumnDef<Coupon>[]),
    ],
    [isCompact, offerName]
  )

  return (
    <StandardTable
      tableKey="coupons"
      columns={columns}
      data={couponsQuery.data?.coupons ?? []}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={couponsQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={couponsQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
      rowClassName={row => (row.original.localId === selectedLocalId ? 'bg-accent/50' : '')}
    />
  )
}
