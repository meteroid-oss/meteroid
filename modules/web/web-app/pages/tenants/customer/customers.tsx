import { Button, Flex } from '@ui/index'
import { Fragment, FunctionComponent, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { EmptyState } from '@/components/empty-state/EmptyState'
import { TenantPageLayout } from '@/components/layouts'
import { CustomersCreatePanel, CustomersHeader, CustomersTable } from '@/features/customers'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Customers: FunctionComponent = () => {
  const isExpress = useIsExpressOrganization()
  const [createPanelVisible, setCreatePanelVisible] = useState(false)
  const [search, setSearch] = useState('')
  const [searchParams] = useSearchParams()

  const currentTab = searchParams.get('tab') || 'active'

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  // Map tab to archived filter
  const archivedFilter =
    currentTab === 'archived' ? true : currentTab === 'active' ? false : undefined

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        perPage: pagination.pageSize,
        page: pagination.pageIndex,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      sortBy: ListCustomerRequest_SortBy.NAME_ASC,
      archived: archivedFilter,
    },
    {}
  )

  const data = customersQuery.data?.customers ?? []
  const count = customersQuery.data?.paginationMeta?.totalItems ?? 0
  const isLoading = customersQuery.isLoading

  const isEmpty = data.length === 0

  return (
    <Fragment>
      <TenantPageLayout>
        <Flex direction="column" className="gap-8 h-full">
          <CustomersHeader
            count={count}
            isLoading={isLoading}
            refetch={() => customersQuery.refetch()}
            setEditPanelVisible={setCreatePanelVisible}
            setSearch={setSearch}
            search={search}
            onImportSuccess={() => customersQuery.refetch()}
          />
          {isEmpty ? (
            <EmptyState
              title="No customers yet"
              description="Create your first customers and assign a subscription"
              imageName="customers"
              actions={
                !isExpress ? (
                  <Button size="sm" variant="default" onClick={() => setCreatePanelVisible(true)}>
                    New customer
                  </Button>
                ) : undefined
              }
            />
          ) : (
            <CustomersTable
              data={data}
              totalCount={count}
              pagination={pagination}
              setPagination={setPagination}
              isLoading={isLoading}
            />
          )}
        </Flex>
      </TenantPageLayout>
      {!isExpress && (
        <CustomersCreatePanel
          visible={createPanelVisible}
          closePanel={() => setCreatePanelVisible(false)}
        />
      )}
    </Fragment>
  )
}
