import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Flex,
  Input,
  Separator,
  Skeleton,
  cn,
} from '@md/ui'
import { Check, ChevronDown, ChevronRight } from 'lucide-react'
import { Fragment, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { PageLayout } from '@/components/layouts/PageLayout'
import { Loader } from '@/features/auth/components/Loader'
import { CustomersEditPanel } from '@/features/customers'
import { CustomerInvoiceModal } from '@/features/customers/modals/CustomerInvoiceModal'
import { CustomerDetailsPanel, CustomerOverviewPanel } from '@/features/customers/panels'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerById, listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'
import { useTypedParams } from '@/utils/params'

export const Customer = () => {
  const { customerId } = useTypedParams<{ customerId: string }>()
  const basePath = useBasePath()
  const navigate = useNavigate()

  const [editPanelVisible, setEditPanelVisible] = useState(false)
  const [createInvoiceVisible, setCreateInvoiceVisible] = useState(false)
  const [search, setSearch] = useState('')

  const debouncedSearch = useDebounceValue(search, 400)

  const customerQuery = useQuery(
    getCustomerById,
    {
      id: customerId ?? '',
    },
    { enabled: Boolean(customerId) }
  )

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        limit: 20,
        offset: 0,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      sortBy: ListCustomerRequest_SortBy.NAME_ASC,
    },
    {}
  )

  const data = customerQuery.data?.customer
  const isLoading = customerQuery.isLoading
  const customersList = customersQuery.data?.customers ?? []
  const isCustomersLoading = customersQuery.isLoading

  return (
    <Fragment>
      <PageLayout
        imgLink="customers"
        title=""
        customTabs={<Flex align="center" className="gap-2">
          <div
            className="text-[15px] font-medium text-muted-foreground cursor-pointer"
            onClick={() => navigate('..')}
          >
            Customers
          </div>
          <ChevronRight size={14} className="text-muted-foreground" />
          <div className="text-[15px] font-medium">{data?.name || data?.alias}</div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Flex
                align="center"
                justify="center"
                className="h-4 w-4 cursor-pointer rounded bg-[#323232]"
              >
                <ChevronDown size={15} className="text-[#76777D]" />
              </Flex>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" alignOffset={-134} sideOffset={5} className="w-56">
              <Input
                value={search}
                onChange={e => setSearch(e.target.value)}
                autoFocus
                placeholder="Search..."
                className="h-7 w-full bg-transparent focus-visible:shadow-none outline-none border-none"
              />
              <DropdownMenuSeparator />
              {isCustomersLoading ? (
                <Loader />
              ) : (
                customersList.map(customer => (
                  <DropdownMenuItem
                    onClick={() => navigate(`${basePath}/customers/${customer.id}`)}
                    key={customer.id}
                    className={cn(customer.name === data?.name && 'bg-accent', 'mt-1 cursor-pointer')}
                  >
                    <Flex align="center" justify="between" className="w-full">
                      {customer.name}
                      {customer.name === data?.name && <Check size={16} />}
                    </Flex>
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </Flex>}
        actions={<>
          <Button size="sm" onClick={() => setCreateInvoiceVisible(true)} variant="secondary">
            Create Invoice
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button hasIcon size="sm" variant="secondary">
                Actions <ChevronDown size={14} className="text-muted-foreground" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[241px]">
              <DropdownMenuItem>Assign subscription</DropdownMenuItem>
              <DropdownMenuItem>Charge one time payment</DropdownMenuItem>
              <DropdownMenuItem onClick={() => setCreateInvoiceVisible(true)}>Create Invoice</DropdownMenuItem>
              <DropdownMenuItem>Create quote</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem>Add balance</DropdownMenuItem>
              <DropdownMenuItem onClick={() => setEditPanelVisible(true)}>
                Edit customer details
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem>Archive customer</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </>}
      >
        <div className="mx-[-16px]">
          <Separator />
        </div>
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <div className="flex h-full">
            <CustomerOverviewPanel
              customer={data}
              onCreateInvoice={() => setCreateInvoiceVisible(true)}
            />
            <CustomerDetailsPanel customer={data} />
          </div>
        )}
      </PageLayout>
      <CustomersEditPanel
        visible={editPanelVisible}
        closePanel={() => setEditPanelVisible(false)}
      />
      <CustomerInvoiceModal openState={[createInvoiceVisible, setCreateInvoiceVisible]} />
    </Fragment>
  )
}
