import { SearchIcon } from '@md/icons'
import { Button, InputWithIcon, Separator } from '@md/ui'
import { Flex } from '@ui/index'
import { ListFilter } from 'lucide-react'
import { Fragment, FunctionComponent, useState } from 'react'

import { EmptyState } from '@/components/empty-state/EmptyState'
import { PageLayout } from '@/components/layouts/PageLayout'
import { CustomersEditPanel, CustomersTable } from '@/features/customers'
import { CustomersExportModal } from '@/features/customers/modals/CustomersExportModal'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Customers: FunctionComponent = () => {
  const [createPanelVisible, setCreatePanelVisible] = useState(false)
  const [search, setSearch] = useState('')
  const [exportModalVisible, setExportModalVisible] = useState(false)

  const debouncedSearch = useDebounceValue(search, 400)

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

  const isEmpty = data.length === 0

  return (
    <Fragment>
      <PageLayout
        imgLink="customers"
        title="Customers"
        tabs={[
          { key: 'all', label: 'All' },
          { key: 'active', label: 'Active' },
          { key: 'inactive', label: 'Inactive' },
          { key: 'archived', label: 'Archived' }
        ]}
        actions={<>
          <Button size="sm" onClick={() => setExportModalVisible(true)} variant="secondary">
            Export
          </Button>
          <Button size="sm" variant="default" onClick={() => setCreatePanelVisible(true)}>
            New customer
          </Button>
        </>}
      >
        <div className="mx-[-16px]">
          <Separator />
        </div>
        <Flex direction="row" align="center" className="gap-4">
          <InputWithIcon
            className="h-[30px]"
            placeholder="Search..."
            icon={<SearchIcon size={16} className="text-[#898784]" />}
            width="fit-content"
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
          <Button
            hasIcon
            className="h-[30px] bg-accent text-accent-foreground hover:opacity-90"
            variant="outline"
          >
            <ListFilter size={16} className="text-[#898784]" /> Filter
          </Button>
        </Flex>
        {isEmpty ? (
          <EmptyState
            title="No customers yet"
            description="Create your first customers and assign a subscription"
            imageName="customers"
            actions={
              <Button size="sm" variant="default" onClick={() => setCreatePanelVisible(true)}>
                New customer
              </Button>
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
      </PageLayout>
      <CustomersEditPanel
        visible={createPanelVisible}
        closePanel={() => setCreatePanelVisible(false)}
      />
      <CustomersExportModal openState={[exportModalVisible, setExportModalVisible]} />
    </Fragment>
  )
}
