import { skipToken } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, ReactNode, useCallback, useMemo, useState } from 'react'
import { Link, Outlet, useNavigate, useSearchParams } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListToolbarSearch, RefreshButton, TableCard } from '@/components/layouts'
import { ListPageHeader } from '@/components/layouts/ListPageLayout'
import { Slot, useModule } from '@/edition'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { PlansTable } from '@/features/plans/PlansTable'
import { LineSelect, PlansLadder, ProductLine } from '@/features/plans/ladder/PlansLadder'
import { Switcher } from '@/features/plans/ladder/Switcher'
import { ladderOf } from '@/features/plans/ladder/ladderOrder'
import { LADDER_PAGE_SIZE } from '@/features/plans/ladder/useAddToPricingPage'
import { PricingPagePreview } from '@/features/plans/pricingpage/PricingPagePreview'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { ALL_OR_LIST_SERDE, SetQueryStateAction, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

// Every plan but the archived ones: the list's baseline, shown without a filter chip.
const DEFAULT_STATUSES = ['active', 'draft', 'inactive']
const DEFAULT_TYPES = ['free', 'standard', 'custom']

type PlansView = 'ladder' | 'pricing' | 'table'
const VIEWS: { value: PlansView; label: string }[] = [
  { value: 'table', label: 'Table' },
  { value: 'ladder', label: 'Ladder' },
  { value: 'pricing', label: 'Pricing page' },
]
const VIEW_SERDE = {
  serialize: (value: PlansView) => value,
  deserialize: (value: string): PlansView =>
    VIEWS.some(view => view.value === value) ? (value as PlansView) : 'table',
}

const STATUS_FIELD = optionField('status', 'Status', [
  { label: 'Active', value: 'active' },
  { label: 'Draft', value: 'draft' },
  { label: 'Inactive', value: 'inactive' },
  { label: 'Archived', value: 'archived' },
])

const sameValues = (a: string[], b: string[]) =>
  a.length === b.length && a.every(value => b.includes(value))

const TYPE_FIELD = optionField('type', 'Type', [
  { label: 'Free', value: 'free' },
  { label: 'Standard', value: 'standard' },
  { label: 'Custom', value: 'custom' },
])

export const Plans: FunctionComponent = () => {
  const navigate = useNavigate()
  const isExpress = useIsExpressOrganization()
  const hasQuotes = !!useModule('cpq')
  const basePath = useBasePath()
  const { has } = usePermissions()
  const canEdit = !isExpress && has('catalog:manage')

  const [view, setView] = useQueryState<PlansView>('view', 'table', {
    ...VIEW_SERDE,
    replace: true,
  })
  // The table filters by it, unset for every line; the ladder and the page show one line.
  const [line, setLine] = useQueryState<string | undefined>('line', undefined)
  const [currency, setCurrency] = useQueryState<string | undefined>('currency', undefined)
  const [option, setOption] = useQueryState<string | undefined>('option', undefined)
  const [, setSearchParams] = useSearchParams()

  const familiesQuery = useQuery(listProductFamilies)
  const lines: ProductLine[] = useMemo(
    () => familiesQuery.data?.productFamilies.map(pf => ({ id: pf.localId, name: pf.name })) ?? [],
    [familiesQuery.data]
  )
  const familyId = line ?? lines[0]?.id
  const lineName = lines.find(l => l.id === familyId)?.name

  const linePlansQuery = useQuery(
    listPlans,
    view !== 'table' && familyId
      ? { productFamilyLocalId: familyId, pagination: { page: 0, perPage: LADDER_PAGE_SIZE } }
      : skipToken
  )
  const linePlans = useMemo(() => linePlansQuery.data?.plans ?? [], [linePlansQuery.data])
  // Whether the tenant has any plan at all: without one, every view is the same empty page.
  const anyPlanQuery = useQuery(listPlans, { pagination: { page: 0, perPage: 1 } })
  const planCount = anyPlanQuery.data?.paginationMeta?.totalItems
  const noPlan = planCount === 0
  const onLadder = ladderOf(linePlans).length

  // Another line sells other currencies and options: its page picks its own. One update, as
  // updates in a row would each start from the same URL.
  const onLine = (id: string) =>
    setSearchParams(prev => {
      const next = new URLSearchParams(prev)
      next.set('line', id)
      next.delete('currency')
      next.delete('option')
      return next
    })

  const subtitle =
    view === 'ladder' && lineName && linePlansQuery.data
      ? `${lineName} · ${onLadder} ${onLadder === 1 ? 'plan' : 'plans'} on the ladder · ${
          linePlans.length - onLadder
        } off it`
      : undefined

  const newPlanButton = (label: string) =>
    !isExpress && (
      <Gate perm="catalog:manage">
        <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-plan')}>
          <AddIcon /> {label}
        </Button>
      </Gate>
    )
  const quoteTemplates = hasQuotes && !isExpress && !noPlan && view !== 'pricing'

  return (
    <div data-full-bleed className="flex min-h-0 flex-1 flex-col">
      <ListPageHeader
        title="Plans"
        tabs={!noPlan && <Switcher label="View" value={view} items={VIEWS} onChange={setView} />}
        actions={
          !isExpress && (
            <Gate perm="catalog:manage">
              <Slot name="plans.listActions" />
              <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-plan')}>
                <AddIcon /> New plan
              </Button>
            </Gate>
          )
        }
      />

      {!noPlan && view === 'table' ? (
        <PlansTableView
          line={line}
          setLine={setLine}
          lines={lines}
          footer={
            quoteTemplates && (
              <span className="text-[13px] text-muted-foreground">
                Selling an enterprise deal?{' '}
                <Link
                  to={`${basePath}/quotes/templates`}
                  className="text-foreground hover:underline"
                >
                  Quote templates
                </Link>
              </span>
            )
          }
        />
      ) : (
        <div className="flex flex-1 flex-col gap-5 px-3 pb-4">
          {subtitle && !noPlan && <p className="text-sm text-muted-foreground">{subtitle}</p>}
          {noPlan ? (
            <EmptyState
              title="No plans yet"
              description="A plan is what customers subscribe to: its products, prices and features."
              scene="plans"
              actions={newPlanButton('Create a plan')}
            />
          ) : !familyId ? (
            <div className="rounded-lg border border-dashed border-border px-4 py-10 text-center text-sm text-muted-foreground">
              {familiesQuery.isLoading ? 'Loading…' : 'No product line yet.'}
            </div>
          ) : view === 'pricing' ? (
            <PricingPagePreview
              familyId={familyId}
              plans={linePlans}
              canEdit={canEdit}
              currency={currency}
              option={option}
              onCurrency={setCurrency}
              onOption={setOption}
              controls={
                lines.length > 1 && <LineSelect lines={lines} value={familyId} onChange={onLine} />
              }
            />
          ) : (
            <PlansLadder
              familyId={familyId}
              lines={lines}
              onLine={onLine}
              plans={linePlans}
              isLoading={linePlansQuery.isLoading}
              canEdit={canEdit}
              currency={currency}
              option={option}
              onCurrency={setCurrency}
              onOption={setOption}
            />
          )}

          {quoteTemplates && (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-dashed border-border px-4 py-3 text-sm">
              <div>
                <div className="font-medium">Selling an enterprise deal?</div>
                <div className="text-muted-foreground">
                  Contracts are quoted from templates, with the terms negotiated.
                </div>
              </div>
              <Button variant="secondary" size="sm" asChild>
                <Link to={`${basePath}/quotes/templates`}>Quote templates</Link>
              </Button>
            </div>
          )}
        </div>
      )}
      <Outlet />
    </div>
  )
}

/** Every plan, filtered and paged. */
const PlansTableView = ({
  line,
  setLine,
  lines,
  footer,
}: {
  line: string | undefined
  setLine: (line: SetQueryStateAction<string | undefined>) => void
  lines: ProductLine[]
  footer?: ReactNode
}) => {
  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useListPagination()

  const [status, setStatus] = useQueryState('status', DEFAULT_STATUSES, ALL_OR_LIST_SERDE)
  const [type, setType] = useQueryState('type', DEFAULT_TYPES, ALL_OR_LIST_SERDE)

  const productFamilyData = useMemo(() => lines.map(l => ({ label: l.name, value: l.id })), [lines])

  const debouncedSearch = useDebounceValue(search, 200)

  const [sorting, setSorting] = useState<SortingState>([])

  useResetPageOnChange(setPagination, [debouncedSearch, status, type, line])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    [setPagination]
  )

  const plansQuery = useQuery(listPlans, {
    productFamilyLocalId: line,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
    orderBy: sortingStateToOrderBy(sorting),
    filters: {
      statuses: status.map(mapPlanStatusToGrpc),
      types: type.map(mapPlanTypeToGrpc),
      search: debouncedSearch,
    },
  })

  return (
    <TableCard
      footer={footer}
      toolbar={
        <>
          <ListToolbarSearch value={search ?? ''} onChange={setSearch} placeholder="Search plans" />
          <FilterBar
            local={[
              // The baseline (all but archived) is no filter: a chip only for another choice.
              {
                field: STATUS_FIELD,
                values: sameValues(status, DEFAULT_STATUSES) ? [] : status,
                set: values => setStatus(values.length > 0 ? values : DEFAULT_STATUSES),
              },
              // Every type is no filter: no chip.
              {
                field: TYPE_FIELD,
                values: type.length === DEFAULT_TYPES.length ? [] : type,
                set: values => setType(values.length > 0 ? values : DEFAULT_TYPES),
              },
              ...(lines.length > 1
                ? [
                    {
                      field: optionField('line', 'Product line', productFamilyData, {
                        multi: false,
                      }),
                      values: line ? [line] : [],
                      set: ([value]: string[]) => setLine(value),
                    },
                  ]
                : []),
            ]}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={plansQuery.isLoading} onClick={() => void plansQuery.refetch()} />
      }
    >
      <PlansTable
        plansQuery={plansQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
        showLine={lines.length > 1}
      />
    </TableCard>
  )
}

function mapPlanStatusToGrpc(s: string): PlanStatus {
  switch (s) {
    case 'active':
      return PlanStatus.ACTIVE
    case 'draft':
      return PlanStatus.DRAFT
    case 'inactive':
      return PlanStatus.INACTIVE
    case 'archived':
      return PlanStatus.ARCHIVED
    default:
      throw new Error(`Unknown status: ${s}`)
  }
}

function mapPlanTypeToGrpc(s: string): PlanType {
  switch (s) {
    case 'free':
      return PlanType.FREE
    case 'standard':
      return PlanType.STANDARD
    case 'custom':
      return PlanType.CUSTOM
    default:
      throw new Error(`Unknown type: ${s}`)
  }
}
