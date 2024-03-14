import { SelectRoot, SelectTrigger, SelectContent, SelectItem, Input } from '@ui/components'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  getCustomer,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'
import { CustomerList } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  value?: CustomerList['id']
  onChange: (id: CustomerList['id']) => void
}

export const CustomerSelect = ({ value, onChange }: Props) => {
  const [search, setSearch] = useState('')

  const getCustomerQuery = useQuery(
    getCustomer,
    {
      id: value ?? '',
    },
    { enabled: Boolean(value) }
  )

  const onValueChange = (value: string) => {
    onChange(value)
    setSearch('')
  }

  const customer = getCustomerQuery.data

  return (
    <SelectRoot value={value} onValueChange={onValueChange}>
      <SelectTrigger className="w-[180px]">{customer ? customer.name : 'Choose one'}</SelectTrigger>
      <SelectContent>
        <Input
          className="mb-2"
          placeholder="Search.."
          autoFocus
          value={search}
          onChange={event => setSearch(event.target.value)}
        />
        <CustomerItems search={search} />
      </SelectContent>
    </SelectRoot>
  )
}

const CustomerItems = ({ search }: { search: string }) => {
  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        limit: 20,
        offset: 0,
      },
      search: search.length > 0 ? search : undefined,
      sortBy: ListCustomerRequest_SortBy.NAME_ASC,
    },
    {}
  )

  const customers = customersQuery.data?.customers || []

  return (
    <>
      {customers.map(c => (
        <SelectItem key={c.id} value={c.id}>
          {c.name}
        </SelectItem>
      ))}
    </>
  )
}
