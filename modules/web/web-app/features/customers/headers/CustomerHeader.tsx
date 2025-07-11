import { spaces } from '@md/foundation'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Flex as NewFlex,
  Separator,
  cn,
} from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { Check, ChevronDown, ChevronRight } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'

interface CustomerHeaderProps {
  name?: string
  setEditPanelVisible: (visible: boolean) => void
  setShowIncoice: () => void
}

export const CustomerHeader: FunctionComponent<CustomerHeaderProps> = ({
  name,
  setEditPanelVisible,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()

  const [search, setSearch] = useState('')

  const debouncedSearch = useDebounceValue(search, 400)

  const pageIndex = 0
  const pageSize = 20

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        perPage: pageSize,
        page: pageIndex,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
      sortBy: ListCustomerRequest_SortBy.NAME_ASC,
    },
    {}
  )

  const data = customersQuery.data?.customers ?? []
  const isLoading = customersQuery.isLoading

  return (
    <>
      <Flex direction="column" gap={spaces.space4}>
        <Flex direction="row" align="center" justify="space-between">
          <NewFlex align="center" className="gap-2">
            <img src="/header/customer.svg" alt="customer logo" />
            <div
              className="text-[15px] font-medium text-muted-foreground cursor-pointer"
              onClick={() => navigate('..')}
            >
              Customers
            </div>
            <ChevronRight size={14} className="text-muted-foreground" />
            <div className="text-[15px] font-medium">{name}</div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <NewFlex
                  align="center"
                  justify="center"
                  className="h-4 w-4 cursor-pointer rounded bg-[#323232]"
                >
                  <ChevronDown size={15} className="text-[#76777D]" />
                </NewFlex>
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
                {isLoading ? (
                  <Loader />
                ) : (
                  data.map(customer => (
                    <DropdownMenuItem
                      onClick={() => navigate(`${basePath}/customers/${customer.id}`)}
                      key={customer.id}
                      className={cn(customer.name === name && 'bg-accent', 'mt-1 cursor-pointer')}
                    >
                      <NewFlex align="center" justify="between" className="w-full">
                        {customer.name}
                        {customer.name === name && <Check size={16} />}
                      </NewFlex>
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </NewFlex>
          <Flex direction="row" gap={spaces.space4}>
            <Button size="sm" variant="secondary" disabled>
              Create Invoice
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button hasIcon size="sm" variant="secondary" disabled>
                  Actions <ChevronDown size={14} className="text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-[241px]">
                <DropdownMenuItem>Assign subscription</DropdownMenuItem>
                <DropdownMenuItem>Charge one time payment</DropdownMenuItem>
                <DropdownMenuItem>Create Invoice</DropdownMenuItem>
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
          </Flex>
        </Flex>
        <div className="mx-[-16px]">
          <Separator />
        </div>
      </Flex>
    </>
  )
}
