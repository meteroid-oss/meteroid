import { Input, Select, SelectContent, SelectItem, SelectTrigger } from '@ui/components'
import { cn } from '@ui/lib'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  getCustomerById,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomerBrief } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  value?: CustomerBrief['id']
  onChange: (id: CustomerBrief['id']) => void
  placeholder?: string
  clearable?: boolean
  /** `filter` matches the toolbar filter buttons (auto width, outline, xs); `default`
   * is the standalone 180px select. */
  variant?: 'default' | 'filter'
  /** Extra classes for the trigger, e.g. to match a compact toolbar height. */
  triggerClassName?: string
}

// Radix Select forbids empty-string item values, so use a sentinel for "no selection".
const ALL_VALUE = '__all__'

export const CustomerSelect = ({
  value,
  onChange,
  placeholder,
  clearable,
  variant = 'default',
  triggerClassName,
}: Props) => {
  const [search, setSearch] = useState('')

  const getCustomerQuery = useQuery(
    getCustomerById,
    {
      id: value ?? '',
    },
    { enabled: Boolean(value) }
  )

  const onValueChange = (next: string) => {
    onChange(next === ALL_VALUE ? '' : next)
    setSearch('')
  }

  const customer = getCustomerQuery.data

  return (
    <Select value={value ?? ''} onValueChange={onValueChange}>
      <SelectTrigger
        className={cn(
          variant === 'filter'
            ? 'h-9 w-auto gap-1 bg-transparent text-xs font-medium'
            : 'w-[180px]',
          triggerClassName
        )}
      >
        {value && customer ? customer.customer?.name : (placeholder ?? 'Choose one')}
      </SelectTrigger>
      <SelectContent>
        <Input
          className="mb-2"
          placeholder="Search.."
          autoFocus
          value={search}
          onChange={event => setSearch(event.target.value)}
        />
        {clearable && (
          <SelectItem value={ALL_VALUE}>{placeholder ?? 'All'}</SelectItem>
        )}
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
