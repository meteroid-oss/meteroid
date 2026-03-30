import { Button, Flex } from '@ui/index'
import { Fragment, FunctionComponent, useCallback, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { EmptyState } from '@/components/empty-state/EmptyState'
import { TenantPageLayout } from '@/components/layouts'
import { CustomersCreatePanel, CustomersHeader, CustomersTable } from '@/features/customers'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { sortingStateToOrderBy } from '@/lib/utils/sorting'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'

import type { PaginationState, SortingState } from '@tanstack/react-table'

export const Customers: FunctionComponent = () => {
  const isExpress = useIsExpressOrganization()
  const [createPanelVisible, setCreatePanelVisible] = useState(false)
  const [search, setSearch] = useState('')
  const [searchParams] = useSearchParams()
  const [sorting, setSorting] = useState<SortingState>([{ id: 'name', desc: false }])

  const currentTab = searchParams.get('tab') || 'active'

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [debouncedSearch, currentTab])

  const handleSortingChange = useCallback(
    (updaterOrValue: SortingState | ((old: SortingState) => SortingState)) => {
      setSorting(prev => (typeof updaterOrValue === 'function' ? updaterOrValue(prev) : updaterOrValue))
      setPagination(prev => ({ ...prev, pageIndex: 0 }))
    },
    []
  )

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
      orderBy: sortingStateToOrderBy(sorting),
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
              sorting={sorting}
              onSortingChange={handleSortingChange}
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
