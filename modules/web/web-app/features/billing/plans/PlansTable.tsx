import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { displayPlanStatus, displayPlanType, printPlanStatus } from '@/features/billing/plans/utils'
import { useQuery } from '@/lib/connectrpc'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { useTypedParams } from '@/utils/params'


import type { FunctionComponent } from 'react'

export const PlansTable: FunctionComponent<{ search: string | undefined }> = ({ search }) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

  const plansQuery = useQuery(listPlans, {
    productFamilyLocalId: familyLocalId!,
    pagination: {
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
    },
    sortBy: ListPlansRequest_SortBy.DATE_DESC,
    filters: {
      statuses: [],
      types: [],
      search: search ? search : undefined,
    },
  })
  const isLoading = plansQuery.isLoading

  const totalCount = plansQuery.data?.paginationMeta?.total ?? 0

  const navigate = useNavigate()

  const columns = useMemo<ColumnDef<PlanOverview>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
        cell: ({ row }) => (
          <span className="flex items-center space-x-2 cursor-pointer">
            <span>{row.original.name}</span>
          </span>
        ),
        enableSorting: false,
      },
      {
        header: 'Version',
        cell: ({ row }) => (
          <span>
            {row.original.activeVersion && <span>{row.original.activeVersion.version}</span>}
          </span>
        ),
      },

      {
        header: 'Status',
        cell: ({ row }) => (
          <span title={printPlanStatus(row.original.planStatus)}>
            {displayPlanStatus(row.original.planStatus)}
          </span>
        ),
      },
      {
        header: 'Type',
        id: 'planType',
        cell: ({ row }) => <>{displayPlanType(row.original.planType)}</>,
      },

      {
        header: 'Description',
        accessorKey: 'description',
        enableSorting: false,
      },
      {
        header: 'Trial',
        id: 'trial',
        cell: ({ row }) => {
          return (
            <>
              {row.original.activeVersion?.trialDurationDays
                ? `${row.original.activeVersion?.trialDurationDays} days`
                : '-'}
            </>
          )
        },
      },

      {
        header: 'Subscriptions',
        accessorFn: c => c.subscriptionCount,
        enableSorting: false,
      },

      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => <LocalId localId={row.original.localId} />,
      },
    ],
    [navigate]
  )

  return (
    <StandardTable
      columns={columns}
      data={plansQuery.data?.plans ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
      isLoading={isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
