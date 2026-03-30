import { useCallback, useEffect, useState } from 'react'

import { SubscriptionsHeader, SubscriptionsTable } from '@/features/subscriptions'
import { useDebounceValue } from '@/hooks/useDebounce'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaginationState, SortingState } from '@tanstack/react-table'

const DEFAULT_STATUSES = ['pending', 'trialing', 'active']

export const Subscriptions = () => {
  const [search, setSearch] = useState('')
  const [sorting, setSorting] = useState<SortingState>([])
  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [statusFilter, setStatusFilter] = useQueryState(
    'status',
    DEFAULT_STATUSES,
    ARRAY_SERDE
  )

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch, statusFilter])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
      status: statusFilter.map(mapSubscriptionStatusToGrpc),
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      orderBy: sortingStateToOrderBy(sorting),
    },
    {}
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = Number(subscriptionsQuery.data?.paginationMeta?.totalItems ?? 0)
  const isLoading = subscriptionsQuery.isLoading

  const refetch = () => {
    subscriptionsQuery.refetch()
  }

  return (
    <div className="flex flex-col gap-8">
      <SubscriptionsHeader
        count={count}
        isLoading={isLoading}
        refetch={refetch}
        statusFilter={statusFilter}
        setStatusFilter={setStatusFilter}
        onImportSuccess={() => subscriptionsQuery.refetch()}
        search={search}
        setSearch={setSearch}
      />
      <SubscriptionsTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        sorting={sorting}
        onSortingChange={handleSortingChange}
      />
    </div>
  )
}

function mapSubscriptionStatusToGrpc(s: string): SubscriptionStatus {
  switch (s) {
    case 'pending':
      return SubscriptionStatus.PENDING
    case 'trialing':
      return SubscriptionStatus.TRIALING
    case 'active':
      return SubscriptionStatus.ACTIVE
    case 'canceled':
      return SubscriptionStatus.CANCELED
    case 'ended':
      return SubscriptionStatus.ENDED
    case 'trial_expired':
      return SubscriptionStatus.TRIAL_EXPIRED
    case 'errored':
      return SubscriptionStatus.ERRORED
    default:
      throw new Error(`Unknown status: ${s}`)
  }
}
