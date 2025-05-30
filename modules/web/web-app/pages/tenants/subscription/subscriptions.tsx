import { SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@ui/components'
import { Flex } from '@ui/index'
import { RefreshCwIcon } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { PageLayout } from '@/components/layouts/PageLayout'
import { SubscriptionsTable } from '@/features/subscriptions'
import { useQuery } from '@/lib/connectrpc'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaginationState } from '@tanstack/react-table'

export const Subscriptions = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
    },
    {}
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = Number(subscriptionsQuery.data?.pagination?.totalItems ?? 0)
  const isLoading = subscriptionsQuery.isLoading

  const refetch = () => {
    subscriptionsQuery.refetch()
  }

  const tabs = [
    { key: 'active', label: 'Active' },
    { key: 'expired', label: 'Expired' },
    { key: 'cancelled', label: 'Cancelled' }
  ]

  return (
    <PageLayout
      imgLink="subscriptions"
      title="Subscriptions"
      tabs={tabs}
      actions={
        <Link to="create">
          <Button size="sm" variant="primary">
            New subscription
          </Button>
        </Link>
      }
    >
      <Flex direction="row" align="center" className="gap-4">
        <InputWithIcon
          placeholder="Search subscriptions"
          icon={<SearchIcon size={16} />}
          width="fit-content"
          disabled
        />
        <Button variant="secondary" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      </Flex>
      <SubscriptionsTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
      />
    </PageLayout>
  )
}
