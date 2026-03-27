import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, CopyIcon, EditIcon, MoreVerticalIcon } from 'lucide-react'
import { FC, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { StandardTable } from '@/components/table/StandardTable'
import { BillableMetricStatusBadge } from '@/features/productCatalog/metrics/BillableMetricStatusBadge'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import {
  archiveBillableMetric,
  listBillableMetrics,
  unarchiveBillableMetric,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  BillableMetricMeta,
} from '@/rpc/api/billablemetrics/v1/models_pb'

export const aggregationTypeMapper: Record<Aggregation_AggregationType, string> = {
  [Aggregation_AggregationType.SUM]: 'sum',
  [Aggregation_AggregationType.MIN]: 'min',
  [Aggregation_AggregationType.MAX]: 'max',
  [Aggregation_AggregationType.MEAN]: 'mean',
  [Aggregation_AggregationType.LATEST]: 'latest',
  [Aggregation_AggregationType.COUNT]: 'count',
  [Aggregation_AggregationType.COUNT_DISTINCT]: 'distinct',
}
interface BillableMetricableProps {
  data: BillableMetricMeta[]
  pagination: PaginationState
  setPagination: OnChangeFn<PaginationState>
  totalCount: number
}
export const BillableMetricTable: FC<BillableMetricableProps> = ({
  data,
  pagination,
  setPagination,
  totalCount,
}) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const isExpress = useIsExpressOrganization()

  const archiveMutation = useMutation(archiveBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      toast.success('Metric archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive metric')
    },
  })

  const unarchiveMutation = useMutation(unarchiveBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      toast.success('Metric unarchived successfully')
    },
    onError: () => {
      toast.error('Failed to unarchive metric')
    },
  })

  const handleArchive = (id: string) => {
    archiveMutation.mutate({ id })
  }

  const handleUnarchive = (id: string) => {
    unarchiveMutation.mutate({ id })
  }

  const columns = useMemo<ColumnDef<BillableMetricMeta>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
        cell: ({ row }) => (
          <button
            onClick={() => navigate(`${row.original.id}`)}
            className="text-left font-medium text-brand hover:underline focus:outline-none"
          >
            {row.original.name}
          </button>
        ),
      },
      {
        header: 'Description',
        accessorKey: 'description',
      },
      {
        header: 'Event name',
        accessorKey: 'code',
      },
      {
        header: 'Aggregation',
        maxSize: 0.1,
        cell: c => (
          <code>
            {aggregationTypeMapper[c.row.original.aggregationType]}
            {c.row.original.aggregationKey && <>({c.row.original.aggregationKey})</>}
          </code>
        ),
      },
      {
        header: 'Status',
        cell: ({ row }) => {
          const isArchived = !!row.original.archivedAt
          return (
            <div className="flex items-center gap-2">
              <BillableMetricStatusBadge isArchived={isArchived} hasSyncError={false} />
            </div>
          )
        },
      },

      ...(!isExpress
        ? [
            {
              accessorKey: 'id' as const,
              header: '',
              maxSize: 0.1,
              cell: ({ row }: { row: { original: BillableMetricMeta } }) => {
                const isArchived = !!row.original.archivedAt
                return (
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="sm">
                        <MoreVerticalIcon size={16} />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      {!isArchived && (
                        <>
                          <DropdownMenuItem onClick={() => navigate(`edit/${row.original.id}`)}>
                            <EditIcon size={16} className="mr-2" />
                            Edit
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onClick={() =>
                              navigate('add-metric', {
                                state: { sourceMetricId: row.original.id },
                              })
                            }
                          >
                            <CopyIcon size={16} className="mr-2" />
                            Duplicate
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                        </>
                      )}
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
      data={data}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={totalCount}
    />
  )
}
