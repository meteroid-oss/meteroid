import { PlusIcon } from '@md/icons'
import { Button } from '@md/ui'
import { SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useMemo, useState } from 'react'
import { Outlet, useNavigate } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { MultiFilter, SingleFilter } from '@/features/TablePage'
import { PlansTable } from '@/features/plans/PlansTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useListPagination, useResetPageOnChange } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

const DEFAULT_STATUSES = ['active', 'draft']
const DEFAULT_TYPES = ['free', 'standard', 'custom']

export const Plans: FunctionComponent = () => {
  const navigate = useNavigate()
  const isExpress = useIsExpressOrganization()
  const productFamiliesQuery = useQuery(listProductFamilies)

  const [search, setSearch] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useListPagination()

  const [status, setStatus] = useQueryState('status', DEFAULT_STATUSES, ARRAY_SERDE)
  const [type, setType] = useQueryState('type', DEFAULT_TYPES, ARRAY_SERDE)
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

  return (
    <ListPageLayout
      title="Plans"
      count={plansQuery.data?.paginationMeta?.totalItems}
      actions={
        isExpress ? undefined : (
          <Gate perm="catalog:manage">
            <Button variant="primary" size="sm" hasIcon onClick={() => navigate('add-plan')}>
              <PlusIcon className="w-4 h-4" /> New plan
            </Button>
          </Gate>
        )
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search ?? ''}
            onChange={setSearch}
            placeholder="Search plans"
            className="max-w-[220px]"
          />
          <MultiFilter
            emptyLabel="All statuses"
            entries={['active', 'draft', 'inactive', 'archived']}
            hook={[status, setStatus]}
          />
          <MultiFilter
            emptyLabel="All types"
            entries={['free', 'standard', 'custom']}
            hook={[type, setType]}
          />
          <SingleFilter
            emptyLabel="All product lines"
            entries={productFamilyData}
            hook={[line, setLine]}
          />
        </>
      }
      toolbarTrailing={
        <RefreshButton isLoading={plansQuery.isLoading} onClick={() => plansQuery.refetch()} />
      }
    >
      <PlansTable
        plansQuery={plansQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
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
