import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useMemo, useState } from 'react'
import { Outlet } from 'react-router-dom'

import { MultiFilter, SingleFilter } from '@/features/TablePage'
import { PlansHeader } from '@/features/plans/PlansHeader'
import { PlansTable } from '@/features/plans/PlansTable'
import { useDebounceValue } from '@/hooks/useDebounce'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const Plans: FunctionComponent = () => {
  const productFamiliesQuery = useQuery(listProductFamilies)

  const [search] = useQueryState<string | undefined>('q', undefined)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const [status, setStatus] = useQueryState('status', ['active', 'draft'], ARRAY_SERDE)
  const [type, setType] = useQueryState('type', ['free', 'standard', 'custom'], ARRAY_SERDE)
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

  const plansQuery = useQuery(listPlans, {
    productFamilyLocalId: line,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
    sortBy: ListPlansRequest_SortBy.DATE_DESC,
    filters: {
      statuses: status.map(mapPlanStatusToGrpc),
      types: type.map(mapPlanTypeToGrpc),
      search: debouncedSearch,
    },
  })

  return (
    <>
      <PlansHeader count={plansQuery.data?.paginationMeta?.totalItems}>
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
      <PlansTable plansQuery={plansQuery} pagination={pagination} setPagination={setPagination} />
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
