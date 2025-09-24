import { Input, Select, SelectContent, SelectItem, SelectTrigger } from '@ui/components'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  getCustomerById,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  value?: CustomerBrief['id']
  onChange: (id: CustomerBrief['id']) => void
  placeholder?: string
}

export const CustomerSelect = ({ value, onChange, placeholder }: Props) => {
  const [search, setSearch] = useState('')

  const getCustomerQuery = useQuery(
    getCustomerById,
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
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger className="w-[180px]">
        {customer ? customer.customer?.name : (placeholder ?? 'Choose one')}
      </SelectTrigger>
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
    </Select>
  )
}

const CustomerItems = ({ search }: { search: string }) => {
  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        perPage: 20,
        page: 0,
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
