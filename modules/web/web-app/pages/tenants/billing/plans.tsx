import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useMemo, useState } from 'react'
import { Outlet, useNavigate } from 'react-router'

import { Gate } from '@/components/Gate'
import { EmptyState } from '@/components/empty-state/EmptyState'
import { AddIcon } from '@/components/icons/AppIcons'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { FilterBar, optionField } from '@/features/filters/FilterBar'
import { PlansTable } from '@/features/plans/PlansTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { ALL_OR_LIST_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

// Every plan but the archived ones: the list's baseline, shown without a filter chip.
const DEFAULT_STATUSES = ['active', 'draft', 'inactive']
const DEFAULT_TYPES = ['free', 'standard', 'custom']

const STATUS_FIELD = optionField('status', 'Status', [
  { label: 'Active', value: 'active' },
  { label: 'Draft', value: 'draft' },
  { label: 'Inactive', value: 'inactive' },
  { label: 'Archived', value: 'archived' },
])

const ALL_STATUSES = ['active', 'draft', 'inactive', 'archived']

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
  const productFamiliesQuery = useQuery(listProductFamilies)

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useListPagination()

  const [status, setStatus] = useQueryState('status', DEFAULT_STATUSES, ALL_OR_LIST_SERDE)
  const [type, setType] = useQueryState('type', DEFAULT_TYPES, ALL_OR_LIST_SERDE)
  const [line, setLine] = useQueryState<string | undefined>('line', undefined)

  const productFamilyData = useMemo(
    () =>
      productFamiliesQuery.data?.productFamilies.map(pf => ({
        label: pf.name,
        value: pf.localId,
      })) ?? [],
    [productFamiliesQuery.data]
  )

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

  // A catalogue with no plan at all gets the empty state. The default view hides archived plans,
  // so when it is empty one plan of any status and type is looked for first.
  const isNarrowed =
    !!debouncedSearch ||
    !!line ||
    !sameValues(status, DEFAULT_STATUSES) ||
    !sameValues(type, DEFAULT_TYPES)
  const defaultViewEmpty =
    !isNarrowed &&
    pagination.pageIndex === 0 &&
    plansQuery.isSuccess &&
    (plansQuery.data?.paginationMeta?.totalItems ?? 0) === 0
  const anyPlanQuery = useQuery(
    listPlans,
    {
      pagination: { page: 0, perPage: 1 },
      filters: {
        statuses: ALL_STATUSES.map(mapPlanStatusToGrpc),
        types: DEFAULT_TYPES.map(mapPlanTypeToGrpc),
      },
    },
    { enabled: defaultViewEmpty }
  )
  const isEmpty =
    defaultViewEmpty &&
    anyPlanQuery.isSuccess &&
    (anyPlanQuery.data?.paginationMeta?.totalItems ?? 0) === 0

  return (
    <ListPageLayout
      title="Plans"
      actions={
        isExpress ? undefined : (
          <Gate perm="catalog:manage">
            <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-plan')}>
              <AddIcon /> New plan
            </Button>
          </Gate>
        )
      }
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
              {
                field: optionField('line', 'Product line', productFamilyData, { multi: false }),
                values: line ? [line] : [],
                set: ([value]) => setLine(value),
              },
            ]}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={plansQuery.isLoading} onClick={() => plansQuery.refetch()} />
      }
    >
      {isEmpty ? (
        <EmptyState
          title="No plans yet"
          description="Plans bundle your products and prices. Create one to start billing your customers."
          scene="plans"
          actions={
            isExpress ? undefined : (
              <Gate perm="catalog:manage">
                <Button size="sm" variant="default" onClick={() => navigate('add-plan')}>
                  New plan
                </Button>
              </Gate>
            )
          }
        />
      ) : (
        <PlansTable
          plansQuery={plansQuery}
          pagination={pagination}
          setPagination={setPagination}
          sorting={sorting}
          onSortingChange={handleSortingChange}
        />
      )}
      <Outlet />
    </ListPageLayout>
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
