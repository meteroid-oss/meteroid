import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { displayPlanStatus, displayPlanType, printPlanStatus } from '@/features/plans/utils'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import { ListPlansResponse } from '@/rpc/api/plans/v1/plans_pb'


import type { FunctionComponent } from 'react'

interface PlansTableProps {
  plansQuery: UseQueryResult<ListPlansResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
}
export const PlansTable: FunctionComponent<PlansTableProps> = ({
  plansQuery,
  pagination,
  setPagination,
}) => {
  const navigate = useNavigate()

  const columns = useMemo<ColumnDef<PlanOverview>[]>(
    () => [
      {
        header: 'Name',
        cell: ({ row }) => <span>{row.original.name}</span>,
        enableSorting: false,
      },
      {
        header: 'Default',
        cell: ({ row }) => (
          <span>
            {row.original.activeVersion ? <span>v{row.original.activeVersion.version}</span> : '-'}
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

      // TODO add created at
      {
        header: 'Subscriptions',
        accessorFn: c => c.subscriptionCount,
        enableSorting: false,
      },
      {
        header: 'Product line',
        id: 'productFamilyName',
        cell: ({ row }) => <>{row.original.productFamilyName}</>,
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
      data={plansQuery.data?.plans ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={plansQuery.data?.paginationMeta?.total ?? 0}
      isLoading={plansQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
