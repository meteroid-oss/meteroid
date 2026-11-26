import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, MoreVerticalIcon, PencilIcon, PowerIcon } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { StandardTable } from '@/components/table/StandardTable'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { KindCell } from '@/features/entitlements/features/featureKind'
import { appliesPerLabel } from '@/features/entitlements/model'
import { productRows } from '@/features/entitlements/productGroups'
import { useResourceTypes } from '@/features/resources/hooks'
import { ListFeaturesResponse } from '@/rpc/api/entitlements/v1/entitlements_pb'
import { Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

interface Props {
  query: UseQueryResult<ListFeaturesResponse>
  pagination: PaginationState
  setPagination: React.Dispatch<React.SetStateAction<PaginationState>>
  onAction: (feature: Feature, target: FeatureStatus) => void
  groupByProduct: boolean
}

export const FeaturesTable = ({
  query,
  pagination,
  setPagination,
  onAction,
  groupByProduct,
}: Props) => {
  const navigate = useNavigate()
  const { byId: typesById, hasTypes } = useResourceTypes()
  const features = query.data?.features
  const totalItems = query.data?.paginationMeta?.totalItems ?? 0

  // One page of a longer list keeps its header: the next page may hold another product.
  const grouped = useMemo(() => {
    const page = features ?? []
    if (!groupByProduct) return { rows: page, groupOf: undefined }
    return productRows(page, f => f.product, { force: totalItems > page.length })
  }, [features, groupByProduct, totalItems])
  const showProduct = !grouped.groupOf && (features ?? []).some(f => f.product)

  const columns = useMemo<ColumnDef<Feature>[]>(
    () => [
      {
        id: 'name',
        header: 'Name',
        cell: ({ row }) => (
          <div className="flex min-w-0 flex-col">
            <span className="truncate text-sm font-medium">{row.original.name}</span>
            <span className="truncate font-mono text-xs text-muted-foreground">{row.original.code}</span>
          </div>
        ),
      },
      {
        id: 'kind',
        header: 'Kind',
        cell: ({ row }) => <KindCell feature={row.original} />,
      },
      ...(hasTypes
        ? [
            {
              id: 'appliesPer',
              header: 'Applies per',
              cell: ({ row }) => (
                <span className="text-sm">{appliesPerLabel(row.original, typesById)}</span>
              ),
            } satisfies ColumnDef<Feature>,
          ]
        : []),
      ...(!showProduct
        ? []
        : [
            {
              id: 'product',
              header: 'Product',
              cell: ({ row }) => (
                <span className="text-sm text-muted-foreground">
                  {row.original.product?.name ?? '—'}
                </span>
              ),
            } satisfies ColumnDef<Feature>,
          ]),
      {
        id: 'status',
        header: 'Status',
        cell: ({ row }) => <FeatureStatusBadge status={row.original.status} />,
      },
      {
        id: 'created_at',
        header: 'Created',
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {parseAndFormatDate(row.original.createdAt)}
          </span>
        ),
      },
      {
        id: 'actions',
        header: '',
        meta: { skipLink: true },
        cell: ({ row }) => {
          const f = row.original
          const archived = f.status === FeatureStatus.ARCHIVED
          return (
            <Gate perm="catalog:manage">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8"
                    aria-label="Actions"
                    onClick={e => e.stopPropagation()}
                  >
                    <MoreVerticalIcon size={16} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" onClick={e => e.stopPropagation()}>
                  {archived ? (
                    <DropdownMenuItem onClick={() => onAction(f, FeatureStatus.ACTIVE)}>
                      <ArchiveRestoreIcon size={16} className="mr-2" />
                      Restore
                    </DropdownMenuItem>
                  ) : (
                    <>
                      <DropdownMenuItem onClick={() => navigate(`edit/${f.id}`)}>
                        <PencilIcon size={16} className="mr-2" />
                        Edit
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onClick={() =>
                          onAction(
                            f,
                            f.status === FeatureStatus.ACTIVE
                              ? FeatureStatus.DISABLED
                              : FeatureStatus.ACTIVE
                          )
                        }
                      >
                        <PowerIcon size={16} className="mr-2" />
                        {f.status === FeatureStatus.ACTIVE ? 'Disable' : 'Enable'}
                      </DropdownMenuItem>
                      <DropdownMenuItem onClick={() => onAction(f, FeatureStatus.ARCHIVED)}>
                        <ArchiveIcon size={16} className="mr-2" />
                        Archive
                      </DropdownMenuItem>
                    </>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            </Gate>
          )
        },
      },
    ],
    [onAction, hasTypes, typesById, navigate, showProduct]
  )

  return (
    <StandardTable
      tableKey="features"
      columns={columns}
      data={grouped.rows}
      pagination={pagination}
      setPagination={setPagination}
      totalCount={query.data?.paginationMeta?.totalItems ?? 0}
      isLoading={query.isLoading}
      emptyMessage="No features"
      rowLink={row => row.original.id}
      groupBy={grouped.groupOf}
    />
  )
}
