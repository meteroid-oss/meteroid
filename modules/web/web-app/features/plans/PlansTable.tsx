import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@md/ui'
import { useQueryClient, UseQueryResult } from '@tanstack/react-query'
import { ColumnDef, OnChangeFn, PaginationState, SortingState } from '@tanstack/react-table'
import { ArchiveIcon, ArchiveRestoreIcon, MoreVerticalIcon, PlusIcon, XIcon } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { StandardTable } from '@/components/table/StandardTable'
import { HeadlinePrice, SoldIn } from '@/features/plans/currencies/PlanPrice'
import { priceLabel } from '@/features/plans/currencies/headline'
import { usePrefetchPlan } from '@/features/plans/hooks/usePlan'
import { canBeRanked } from '@/features/plans/ladder/ladderOrder'
import {
  useAddToPricingPage,
  useRemoveFromPricingPage,
} from '@/features/plans/ladder/useAddToPricingPage'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { PlanOverview, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
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
  sorting?: SortingState
  onSortingChange?: OnChangeFn<SortingState>
  /** With one product line only, its column is left out. */
  showLine?: boolean
}
export const PlansTable: FunctionComponent<PlansTableProps> = ({
  plansQuery,
  pagination,
  setPagination,
  sorting,
  onSortingChange,
  showLine = true,
}) => {
  const queryClient = useQueryClient()
  const isExpress = useIsExpressOrganization()
  const prefetchPlan = usePrefetchPlan()
  const addToPricingPage = useAddToPricingPage()
  const removeFromPricingPage = useRemoveFromPricingPage()

  const archiveMutation = useMutation(archivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
      toast.success('Plan archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive plan')
    },
  })

  const unarchiveMutation = useMutation(unarchivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
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
        header: 'Plan',
        enableSorting: true,
        cell: ({ row }) => {
          const plan = row.original
          const kind = planKind(plan)
          return (
            <div className={box}>
              <div className="truncate font-medium">{plan.name}</div>
              {(kind || plan.description) && (
                <div className="max-w-xs truncate text-xs text-muted-foreground">
                  {[kind, plan.description].filter(Boolean).join(' · ')}
                </div>
              )}
            </div>
          )
        },
      },
      {
        header: 'Price',
        enableSorting: false,
        cell: ({ row }) => (
          <div className={`${box} max-w-56`}>
            <HeadlinePrice priced={row.original.pricedOptions} options={row.original.options} />
          </div>
        ),
      },
      {
        header: 'Sold in',
        enableSorting: false,
        cell: ({ row }) => (
          <div className={box}>
            <SoldIn priced={row.original.pricedOptions} />
            {!!row.original.activeVersion?.trialDurationDays && (
              <div className="text-xs text-muted-foreground">
                {row.original.activeVersion.trialDurationDays}-day trial
              </div>
            )}
          </div>
        ),
      },
      {
        header: 'Subscriptions',
        enableSorting: false,
        cell: ({ row }) => (
          <div className={`${box} tabular-nums`}>{row.original.subscriptionCount}</div>
        ),
      },
      {
        id: 'pricingPage',
        header: 'Pricing page',
        enableSorting: false,
        cell: ({ row }) =>
          row.original.selfServiceRank !== undefined ? (
            <span className={`${box} whitespace-nowrap`}>
              #{row.original.selfServiceRank}
              <span className="text-muted-foreground">
                {' · '}
                {row.original.salesLed ? 'Contact sales' : 'Self-serve'}
              </span>
            </span>
          ) : (
            <span className={`${box} text-muted-foreground`}>—</span>
          ),
      },
      {
        header: 'Version',
        enableSorting: false,
        cell: ({ row }) => (
          <div className={`${box} whitespace-nowrap`}>
            {row.original.activeVersion ? `v${row.original.activeVersion.version}` : '—'}
            {row.original.hasDraftVersion && (
              <div
                className="text-xs text-muted-foreground"
                title="A new version is being drafted"
              >
                {row.original.activeVersion
                  ? `v${row.original.activeVersion.version + 1} in draft`
                  : 'In draft'}
              </div>
            )}
          </div>
        ),
      },
      {
        id: 'status',
        header: 'Status',
        enableSorting: true,
        cell: ({ row }) => (
          <div className={box}>
            <StatusDot status={row.original.planStatus} />
          </div>
        ),
      },
      ...(showLine
        ? [
            {
              header: 'Product line',
              id: 'productFamilyName',
              cell: ({ row }) => <div className={box}>{row.original.productFamilyName}</div>,
              enableSorting: false,
            } satisfies ColumnDef<PlanOverview>,
          ]
        : []),

      {
        header: 'API Handle',
        id: 'localId',
        cell: ({ row }) => (
          <div className={`${box} font-mono text-xs text-muted-foreground`}>
            {row.original.localId}
          </div>
        ),
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
                  <Gate perm="catalog:manage">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="sm">
                          <MoreVerticalIcon size={16} />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        {row.original.selfServiceRank !== undefined ? (
                          <DropdownMenuItem
                            onClick={() => void removeFromPricingPage(row.original)}
                          >
                            <XIcon size={16} className="mr-2" />
                            Remove from the pricing page
                          </DropdownMenuItem>
                        ) : (
                          canBeRanked(row.original) && (
                            <DropdownMenuItem onClick={() => void addToPricingPage(row.original)}>
                              <PlusIcon size={16} className="mr-2" />
                              Add to the pricing page
                            </DropdownMenuItem>
                          )
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
                  </Gate>
                )
              },
            },
          ]
        : []),
    ],
    [isExpress, addToPricingPage, removeFromPricingPage, showLine]
  )

  return (
    <StandardTable
      tableKey="plans"
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
      onRowIntent={row => prefetchPlan(row.original, plansQuery.dataUpdatedAt)}
    />
  )
}

// Every cell centred on the row's height, whatever its number of lines.
const box = 'flex min-h-10 flex-col justify-center text-sm'

/** A plan type worth naming under its name. */
const KIND: Partial<Record<PlanType, string>> = {
  [PlanType.FREE]: 'Free',
  [PlanType.CUSTOM]: 'Custom',
}

/** Its type, or Free when every option it sells reads free: a standard plan priced at nothing.
 * None sold (included products, or charges the list can't read) says nothing. */
export const planKind = (plan: PlanOverview): string | undefined =>
  KIND[plan.planType] ??
  (plan.free ||
  (plan.pricedOptions.length > 0 && plan.pricedOptions.every(p => priceLabel(p) === 'Free'))
    ? 'Free'
    : undefined)

const STATUS: Record<PlanStatus, { label: string; dot: string }> = {
  [PlanStatus.ACTIVE]: { label: 'Active', dot: 'bg-success' },
  [PlanStatus.DRAFT]: { label: 'Draft', dot: 'bg-muted-foreground' },
  [PlanStatus.INACTIVE]: { label: 'Inactive', dot: 'bg-warning' },
  [PlanStatus.ARCHIVED]: { label: 'Archived', dot: 'bg-muted-foreground/50' },
}

/** A quiet status: a coloured dot and its word, readable in a dense list. */
const StatusDot = ({ status }: { status: PlanStatus }) => {
  const { label, dot } = STATUS[status] ?? { label: '—', dot: 'bg-muted-foreground' }
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <span aria-hidden className={`size-1.5 rounded-full ${dot}`} />
      {label}
    </span>
  )
}
