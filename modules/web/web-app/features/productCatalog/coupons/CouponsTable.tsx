import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { ListCouponResponse } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { parseAndFormatDate, parseAndFormatDateOptional } from '@/utils/date'
import { useTypedParams } from '@/utils/params'

import type { FunctionComponent } from 'react'

interface CouponsTableProps {
  couponsQuery: UseQueryResult<ListCouponResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
}
export const CouponsTable: FunctionComponent<CouponsTableProps> = ({
  couponsQuery,
  pagination,
  setPagination,
}) => {
  const navigate = useNavigate()

  const { couponLocalId } = useTypedParams<{ couponLocalId: string }>()

  const isCompact = couponLocalId !== undefined

  const columns = useMemo<ColumnDef<Coupon>[]>(
    () => [
      {
        header: 'Code',
        cell: ({ row }) => <span>{row.original.code}</span>,
        enableSozrting: false,
      },

      {
        header: 'Redemptions',
        cell: ({ row }) => (
          <>
            <span>{row.original.redemptionCount}</span>
            <span className="text-muted-foreground"> / {row.original.redemptionLimit ?? '∞'}</span>
          </>
        ),
      },
      {
        header: 'Expiry',
        cell: ({ row }) => <span>{parseAndFormatDateOptional(row.original.expiresAt)}</span>,
      },
      ...((isCompact
        ? []
        : [
            {
              header: 'Description',
              cell: ({ row }) => <span className="text-ellipsis">{row.original.description}</span>,
              enableSorting: false,
            },
            {
              header: 'Created at',
              cell: ({ row }) => <span>{parseAndFormatDate(row.original.createdAt)}</span>,
              enableSorting: false,
            },
          ]) as ColumnDef<Coupon>[]),
      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => (
          <LocalId localId={row.original.localId} className={isCompact ? 'max-w-10' : 'max-w-16'} />
        ),
      },
    ],
    [navigate, isCompact]
  )

  return (
    <StandardTable
      columns={columns}
      data={couponsQuery.data?.coupons ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={couponsQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={couponsQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
      rowClassName={row =>
        row.original.localId === couponLocalId ? 'bg-accent/50 font-semibold' : ''
      }
    />
  )
}
