import { spaces } from '@md/foundation'
import { Flex } from '@ui/components'
import { Fragment, FunctionComponent, useState } from 'react'

import { TenantPageLayout } from '@/components/layouts'
import { SubscriptionsHeader, SubscriptionsTable } from '@/features/subscriptions'
import { useQuery } from '@/lib/connectrpc'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaginationState } from '@tanstack/react-table'

export const Subscriptions: FunctionComponent = () => {
  const [, setEditPanelVisible] = useState(false)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      pagination: {
        limit: pagination.pageSize,
        offset: pagination.pageIndex * pagination.pageSize,
      },
    },
    {}
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = subscriptionsQuery.data?.paginationMeta?.total ?? 0
  const isLoading = subscriptionsQuery.isLoading

  const refetch = () => {
    subscriptionsQuery.refetch()
  }

  return (
    <Fragment>
      <TenantPageLayout title="Subscriptions">
        <Flex direction="column" gap={spaces.space9}>
          <SubscriptionsHeader
            count={count}
            setEditPanelVisible={setEditPanelVisible}
            isLoading={isLoading}
            refetch={refetch}
          />
          <SubscriptionsTable
            data={data}
            totalCount={count}
            pagination={pagination}
            setPagination={setPagination}
            isLoading={isLoading}
          />
        </Flex>
      </TenantPageLayout>
    </Fragment>
  )
}
