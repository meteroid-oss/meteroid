import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { MoreVerticalIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { StandardTable } from '@/components/table/StandardTable'
import { useQuery } from '@/lib/connectrpc'
import { Plan } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { useTypedParams } from '@/utils/params'

import type { FunctionComponent } from 'react'

export const PlansTable: FunctionComponent<{ search: string | undefined }> = ({ search }) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()

  const plansQuery = useQuery(listPlans, {
    productFamilyExternalId: familyExternalId!,
    pagination: {
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
    },
    sortBy: ListPlansRequest_SortBy.DATE_DESC,
    search: search ? search : undefined,
  })
  const isLoading = plansQuery.isLoading

  const totalCount = plansQuery.data?.paginationMeta?.total ?? 0

  const navigate = useNavigate()

  const columns = useMemo<ColumnDef<Plan>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
        cell: ({ row }) => (
          <span
            className="flex items-center space-x-2 cursor-pointer"
            onClick={() => navigate(row.original.externalId)}
          >
            <span>{row.original.name}</span>
          </span>
        ),
      },

      {
        header: 'Active subscriptions',
        accessorFn: () => '-',
      },
      {
        header: 'Api Name',
        accessorKey: 'externalId',
      },
      {
        header: 'Description',
        accessorKey: 'description',
      },
      {
        accessorKey: 'id',
        header: '',
        maxSize: 0.1,
        cell: () => <MoreVerticalIcon size={16} className="cursor-pointer" />,
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
    />
  )
}
