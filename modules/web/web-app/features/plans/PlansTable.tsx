import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { useQueryClient , UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, MoreVerticalIcon } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { displayPlanStatus, displayPlanType, printPlanStatus } from '@/features/plans/utils'
import { PlanOverview, PlanStatus } from '@/rpc/api/plans/v1/models_pb'
import {
  archivePlan,
  listPlans,
  unarchivePlan,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
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
  const queryClient = useQueryClient()

  const archiveMutation = useMutation(archivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
      toast.success('Plan archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive plan')
    },
  })

  const unarchiveMutation = useMutation(unarchivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
      toast.success('Plan unarchived successfully')
    },
    onError: () => {
      toast.error('Failed to unarchive plan')
    },
  })

  const handleArchive = (id: string) => {
    archiveMutation.mutate({ id })
  }

  const handleUnarchive = (id: string) => {
    unarchiveMutation.mutate({ id })
  }

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
        cell: ({ row }) => {
          const isArchived = row.original.planStatus === PlanStatus.ARCHIVED
          return (
            <Badge variant={isArchived ? 'secondary' : 'success'} title={printPlanStatus(row.original.planStatus)}>
              {displayPlanStatus(row.original.planStatus)}
            </Badge>
          )
        },
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

      {
        accessorKey: 'id',
        header: '',
        maxSize: 0.1,
        cell: ({ row }) => {
          const isArchived = row.original.planStatus === PlanStatus.ARCHIVED
          return (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm">
                  <MoreVerticalIcon size={16} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {isArchived ? (
                  <DropdownMenuItem onClick={() => handleUnarchive(row.original.id)}>
                    <ArchiveRestoreIcon size={16} className="mr-2" />
                    Unarchive
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem onClick={() => handleArchive(row.original.id)}>
                    <ArchiveIcon size={16} className="mr-2" />
                    Archive
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          )
        },
      },
    ],
    [navigate, handleArchive, handleUnarchive]
  )

  return (
    <StandardTable
      columns={columns}
      data={plansQuery.data?.plans ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={plansQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={plansQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
