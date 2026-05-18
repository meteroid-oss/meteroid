import {
  Badge,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { ArchiveRestoreIcon, MoreVertical, PowerIcon } from 'lucide-react'
import { useMemo } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { featureKindFromProto, featureTypeLabel } from '@/features/entitlements/utils'
import { ListFeaturesResponse } from '@/rpc/api/entitlements/v1/entitlements_pb'
import { Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

interface Props {
  query: UseQueryResult<ListFeaturesResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  onStatusAction: (feature: Feature, target: FeatureStatus) => void
}

export const FeaturesTable = ({
  query,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  onStatusAction,
}: Props) => {
  const columns = useMemo<ColumnDef<Feature>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        enableSorting: true,
        cell: ({ row }) => (
          <div>
            <span className="font-medium">{row.original.name}</span>
            {row.original.description && (
              <span className="block text-xs text-muted-foreground truncate max-w-xs">
                {row.original.description}
              </span>
            )}
          </div>
        ),
      },
      {
        id: 'product',
        header: 'Product',
        enableSorting: false,
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {row.original.product?.name ?? '—'}
          </span>
        ),
      },
      {
        id: 'type',
        header: 'Type',
        enableSorting: false,
        cell: ({ row }) => (
          <Badge variant="secondary">{featureTypeLabel(featureKindFromProto(row.original.featureType))}</Badge>
        ),
      },
      {
        id: 'status',
        header: 'Status',
        enableSorting: false,
        cell: ({ row }) => <FeatureStatusBadge status={row.original.status} />,
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
        id: 'actions',
        header: '',
        enableSorting: false,
        meta: { skipLink: true },
        cell: ({ row }) => {
          const f = row.original
          return (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  className="p-1.5 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
                  onClick={e => e.stopPropagation()}
                >
                  <MoreVertical size={14} />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" onClick={e => e.stopPropagation()}>
                {f.status === FeatureStatus.ACTIVE && (
                  <DropdownMenuItem onClick={() => onStatusAction(f, FeatureStatus.DISABLED)}>
                    <PowerIcon size={14} className="mr-2" /> Disable
                  </DropdownMenuItem>
                )}
                {f.status !== FeatureStatus.ACTIVE && (
                  <DropdownMenuItem onClick={() => onStatusAction(f, FeatureStatus.ACTIVE)}>
                    <ArchiveRestoreIcon size={14} className="mr-2" /> Re-activate
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          )
        },
      },
    ],
    [onStatusAction]
  )

  return (
    <StandardTable
      columns={columns}
      data={query.data?.features ?? []}
      sortable={true}
      sorting={sorting}
      onSortingChange={onSortingChange}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={query.data?.paginationMeta?.totalItems ?? 0}
      isLoading={query.isLoading}
      rowLink={row => row.original.id}
    />
  )
}
