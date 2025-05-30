import { SearchIcon } from '@md/icons'
import { Button, InputWithIcon } from '@md/ui'
import { Flex } from '@ui/index'
import { RefreshCwIcon } from 'lucide-react'
import { Fragment, useState } from 'react'

import { PageLayout } from '@/components/layouts/PageLayout'
import { InvoicesTable } from '@/features/invoices'
import { FilterDropdown } from '@/features/invoices/FilterDropdown'
import { InvoicesSearch } from '@/features/invoices/types'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { ListInvoicesRequest_SortBy } from '@/rpc/api/invoices/v1/invoices_pb'

import type { PaginationState } from '@tanstack/react-table'

export const Invoices = () => {
  const [_, setEditPanelVisible] = useState(false)
  const [search, setSearch] = useState<InvoicesSearch>({})

  const debouncedSearch = useDebounceValue(search, 400)

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 20,
  })

  const invoicesQuery = useQuery(
    listInvoices,
    {
      sortBy: ListInvoicesRequest_SortBy.DATE_DESC,
      search: debouncedSearch.text || '',
      status: debouncedSearch.status,
      pagination: {
        limit: pagination.pageSize,
        offset: pagination.pageIndex * pagination.pageSize,
      },
    },
    {}
  )

  const data = invoicesQuery.data?.invoices ?? []
  const count = data.length
  const isLoading = invoicesQuery.isLoading

  const refetch = () => {
    invoicesQuery.refetch()
  }

  const tabs = [
    { key: 'active', label: 'Active' },
    { key: 'expired', label: 'Expired' },
    { key: 'cancelled', label: 'Cancelled' }
  ]

  return (
    <Fragment>
      <PageLayout
        imgLink="invoices"
        title="Invoices"
        tabs={tabs}
        actions={
          <Button variant="primary" hasIcon onClick={() => setEditPanelVisible(true)} size="sm">
            New invoice
          </Button>
        }
      >
        <Flex direction="row" align="center" className="gap-4">
          <InputWithIcon
            placeholder="Search by customer"
            icon={<SearchIcon size={16} />}
            width="fit-content"
            value={search.text}
            onChange={e => setSearch({ ...search, text: e.target.value })}
          />
          <Button variant="secondary" disabled={isLoading} onClick={refetch}>
            <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
          </Button>
          <FilterDropdown
            status={search.status}
            setStatus={value => setSearch({ ...search, status: value })}
          />
        </Flex>
        <InvoicesTable
          data={data}
          totalCount={count}
          pagination={pagination}
          setPagination={setPagination}
          isLoading={isLoading}
        />
      </PageLayout>
    </Fragment>
  )
}
