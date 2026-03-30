import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { useQueryClient, UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, MoreVerticalIcon } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { toast } from 'sonner'

import { LocalId } from '@/components/LocalId'
import { StandardTable } from '@/components/table/StandardTable'
import { PlanStatusBadge } from '@/features/plans/PlanStatusBadge'
import { displayPlanType } from '@/features/plans/utils'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { PlanOverview, PlanStatus } from '@/rpc/api/plans/v1/models_pb'
import {
  archivePlan,
  listPlans,
  unarchivePlan,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansResponse } from '@/rpc/api/plans/v1/plans_pb'
import { parseAndFormatDate } from '@/utils/date'

import type { FunctionComponent } from 'react'

interface PlansTableProps {
  plansQuery: UseQueryResult<ListPlansResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
}
export const PlansTable: FunctionComponent<PlansTableProps> = ({
  plansQuery,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
}) => {
  const queryClient = useQueryClient()
  const isExpress = useIsExpressOrganization()

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

  const handleArchive = useCallback((id: string) => {
    archiveMutation.mutate({ id })
  }, [])

  const handleUnarchive = useCallback((id: string) => {
    unarchiveMutation.mutate({ id })
  }, [])

  const columns = useMemo<ColumnDef<PlanOverview>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        accessorKey: 'name',
        cell: ({ row }) => <span>{row.original.name}</span>,
      },
      {
        header: 'Default',
        cell: ({ row }) => (
          <span>
            {row.original.activeVersion ? <span>v{row.original.activeVersion.version}</span> : '-'}
          </span>
        ),
        enableSorting: false,
      },

      {
        id: 'status',
        header: 'Status',
        enableSorting: true,
        cell: ({ row }) => <PlanStatusBadge status={row.original.planStatus} />,
      },
      {
        header: 'Type',
        id: 'plan_type',
        enableSorting: true,
        cell: ({ row }) => <>{displayPlanType(row.original.planType)}</>,
      },

      {
        id: 'created_at',
        header: 'Created',
        enableSorting: true,
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {parseAndFormatDate(row.original.createdAt)}
          </span>
        ),
      },
      {
        header: 'Subscriptions',
        accessorFn: c => c.subscriptionCount,
        enableSorting: false,
      },
      {
        header: 'Product line',
        id: 'productFamilyName',
        cell: ({ row }) => <>{row.original.productFamilyName}</>,
        enableSorting: false,
      },

      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => <LocalId localId={row.original.localId} className="max-w-16" />,
        enableSorting: false,
      },

      ...(!isExpress
        ? [
            {
              accessorKey: 'id',
              header: '',
              maxSize: 0.1,
              enableSorting: false,
              cell: ({ row }: { row: { original: PlanOverview } }) => {
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
          ]
        : []),
    ],
    [isExpress]
  )

  return (
    <StandardTable
      columns={columns}
      data={plansQuery.data?.plans ?? []}
      sortable={true}
      pagination={pagination}
      setPagination={setPagination}
      sorting={sorting}
      onSortingChange={onSortingChange}
      totalCount={plansQuery.data?.paginationMeta?.totalItems ?? 0}
      isLoading={plansQuery.isLoading}
      rowLink={row => `${row.original.localId}`}
    />
  )
}
