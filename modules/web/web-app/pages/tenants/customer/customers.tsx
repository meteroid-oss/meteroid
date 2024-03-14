import { spaces } from '@md/foundation'
import { Flex } from '@ui/components/legacy'
import { Fragment, FunctionComponent, useState } from 'react'

import { TenantPageLayout } from '@/components/layouts'
import { CustomersEditPanel, CustomersHeader, CustomersTable } from '@/features/customers'
import useDebounce from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Customers: FunctionComponent = () => {
  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [search, setSearch] = useState('')

  const debouncedSearch = useDebounce(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        limit: pagination.pageSize,
        offset: pagination.pageIndex * pagination.pageSize,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      sortBy: ListCustomerRequest_SortBy.NAME_ASC,
    },
    {}
  )

  const data = customersQuery.data?.customers ?? []
  const count = customersQuery.data?.paginationMeta?.total ?? 0
  const isLoading = customersQuery.isLoading

  const refetch = () => {
    customersQuery.refetch()
  }

  return (
    <Fragment>
      <TenantPageLayout title="Customers">
        <Flex direction="column" gap={spaces.space9}>
          <CustomersHeader
            count={count}
            setEditPanelVisible={setEditPanelVisible}
            isLoading={isLoading}
            refetch={refetch}
            setSearch={setSearch}
            search={search}
          />
          <CustomersTable
            data={data}
            totalCount={count}
            pagination={pagination}
            setPagination={setPagination}
            isLoading={isLoading}
          />
        </Flex>
      </TenantPageLayout>
      <CustomersEditPanel
        visible={editPanelVisible}
        closePanel={() => setEditPanelVisible(false)}
      />
    </Fragment>
  )
}
