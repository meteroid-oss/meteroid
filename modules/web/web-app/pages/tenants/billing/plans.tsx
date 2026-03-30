import { PaginationState, SortingState } from '@tanstack/react-table'
import { FunctionComponent, useCallback, useEffect, useMemo, useState } from 'react'
import { Outlet } from 'react-router-dom'

import { MultiFilter, SingleFilter } from '@/features/TablePage'
import { PlansHeader } from '@/features/plans/PlansHeader'
import { PlansTable } from '@/features/plans/PlansTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

const DEFAULT_STATUSES = ['active', 'draft']
const DEFAULT_TYPES = ['free', 'standard', 'custom']

export const Plans: FunctionComponent = () => {
  const productFamiliesQuery = useQuery(listProductFamilies)

  const [search] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

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

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch, status, type, line])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev =>
        typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue
      )
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
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
    <>
      <PlansHeader
        count={plansQuery.data?.paginationMeta?.totalItems}
        isLoading={plansQuery.isLoading}
        refetch={() => plansQuery.refetch()}
      >
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
      </PlansHeader>
      <PlansTable
        plansQuery={plansQuery}
        pagination={pagination}
        setPagination={setPagination}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
      <Outlet />
    </>
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
