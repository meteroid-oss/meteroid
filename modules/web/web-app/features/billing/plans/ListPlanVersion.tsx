import { StandardTable } from '@/components/table/StandardTable'
import { usePlanOverview } from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { ListPlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPlanVersionById } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { disableQuery } from '@connectrpc/connect-query'
import { ColumnDef, PaginationState, Row } from '@tanstack/react-table'
import { useMemo, useState } from 'react'

export const ListPlanVersionTab = () => {
  const overview = usePlanOverview()

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 15,
  })

  const planVersions = useQuery(
    listPlanVersionById,
    overview
      ? {
          planId: overview.planId,
          pagination: {
            limit: pagination.pageSize,
            offset: pagination.pageIndex * pagination.pageSize,
          },
        }
      : disableQuery
  )

  const data = planVersions.data?.planVersions ?? []
  const count = Number(planVersions.data?.paginationMeta?.total ?? 0)
  const isLoading = planVersions.isLoading

  const columns = useMemo<ColumnDef<ListPlanVersion>[]>(
    () => [
      {
        header: 'Version',
        accessorKey: 'version',
      },

      {
        header: 'Status',
        cell: ({ row }: { row: Row<ListPlanVersion> }) =>
          row.original.isDraft ? 'Draft' : 'Active',
      },
    ],

    []
  )

  return (
    <StandardTable
      columns={columns}
      data={data}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={count}
      isLoading={isLoading}
    />
  )
}
