import { useState } from 'react'

import { SubscriptionsHeader, SubscriptionsTable } from '@/features/subscriptions'
import { ARRAY_SERDE, useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaginationState } from '@tanstack/react-table'

export const Subscriptions = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })
  const [statusFilter, setStatusFilter] = useQueryState(
    'status',
    ['pending', 'trialing', 'active'],
    ARRAY_SERDE
  )

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
      status: statusFilter.map(mapSubscriptionStatusToGrpc),
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
      />
      <SubscriptionsTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
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
