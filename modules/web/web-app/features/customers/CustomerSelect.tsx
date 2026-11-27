import { keepPreviousData } from '@tanstack/react-query'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@ui/components'
import { cn } from '@ui/lib'
import { Check, ChevronsUpDown } from 'lucide-react'
import { useState } from 'react'

import { InitialAvatar } from '@/components/icons/AppIcons'
import { useDebounceValue } from '@/hooks/useDebounce'
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
   * is the standalone 180px select; `card` fills its field and shows the chosen customer's
   * initial, name and billing email. */
  variant?: 'default' | 'filter' | 'card'
  /** Extra classes for the trigger, e.g. to match a compact toolbar height. */
  triggerClassName?: string
}

// The "no customer" entry of a clearable picker; cmdk items need a value of their own.
const NONE_VALUE = '__none__'

/**
 * Picks a customer, searching them as you type. A combobox (a Command in a Popover) rather than a
 * Select: the search keeps the focus while the list changes under it, and stays above the list
 * as it scrolls.
 */
export const CustomerSelect = ({
  value,
  onChange,
  placeholder,
  clearable,
  variant = 'default',
  triggerClassName,
}: Props) => {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')

  const getCustomerQuery = useQuery(
    getCustomerById,
    { id: value ?? '' },
    { enabled: Boolean(value) }
  )
  const chosen = value ? getCustomerQuery.data?.customer : undefined

  const choose = (next: string) => {
    onChange(next === NONE_VALUE ? '' : next)
    setOpen(false)
    setSearch('')
  }

  return (
    <Popover
      open={open}
      onOpenChange={next => {
        setOpen(next)
        if (!next) setSearch('')
      }}
    >
      <PopoverTrigger asChild>
        {/* Drawn as a text field (see Input and SelectTrigger). */}
        <button
          type="button"
          role="combobox"
          aria-expanded={open}
          className={cn(
            'flex h-8 items-center justify-between gap-2.5 whitespace-nowrap rounded-md border border-field-border bg-field px-3 py-1 text-left text-[13px] font-medium text-field-foreground transition-[border-color,box-shadow] focus:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8 disabled:cursor-not-allowed disabled:opacity-50',
            variant === 'filter'
              ? 'h-9 w-auto gap-1 bg-transparent text-xs'
              : variant === 'card'
                ? cn('w-full', chosen && 'h-auto py-2.5 pl-2.5')
                : 'w-[180px]',
            triggerClassName
          )}
        >
          {variant === 'card' && chosen ? (
            <span className="flex min-w-0 items-center gap-3">
              <InitialAvatar name={chosen.name} className="size-7 rounded-md text-xs" />
              <span className="flex min-w-0 flex-col gap-1 leading-none">
                <span className="truncate font-medium text-foreground">{chosen.name}</span>
                {chosen.billingEmail && (
                  <span className="truncate text-xs font-normal text-muted-foreground">
                    {chosen.billingEmail}
                  </span>
                )}
              </span>
            </span>
          ) : (
            <span className={cn('truncate', !chosen && 'text-field-placeholder')}>
              {chosen?.name ?? placeholder ?? 'Choose one'}
            </span>
          )}
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-field-placeholder" aria-hidden />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] min-w-60 p-0">
        {/* The server searches: cmdk shows the results as they come, without filtering them. */}
        <Command shouldFilter={false}>
          <CommandInput placeholder="Search customers…" value={search} onValueChange={setSearch} />
          <CommandList>
            {open && (
              <CustomerItems
                search={search}
                value={value}
                onSelect={choose}
                none={clearable ? (placeholder ?? 'All') : undefined}
              />
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

const CustomerItems = ({
  search,
  value,
  onSelect,
  none,
}: {
  search: string
  value?: string
  onSelect: (id: string) => void
  /** The label of the "no customer" entry, when the picker can be cleared. */
  none?: string
}) => {
  const debouncedSearch = useDebounceValue(search.trim(), 150)
  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: { perPage: 20, page: 0 },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
    },
    // The last results stay while the next ones load, so the list doesn't flash empty.
    { placeholderData: keepPreviousData }
  )
  const customers = customersQuery.data?.customers ?? []

  return (
    <>
      {!customersQuery.isLoading && <CommandEmpty>No customer found.</CommandEmpty>}
      <CommandGroup>
        {none && (
          <CommandItem value={NONE_VALUE} onSelect={onSelect}>
            <span className="truncate">{none}</span>
            {!value && <Check className="ml-auto h-3.5 w-3.5 text-menu-foreground" />}
          </CommandItem>
        )}
        {customers.map(customer => (
          <CommandItem key={customer.id} value={customer.id} onSelect={onSelect}>
            <span className="truncate">{customer.name}</span>
            {customer.id === value && (
              <Check className="ml-auto h-3.5 w-3.5 text-menu-foreground" />
            )}
          </CommandItem>
        ))}
      </CommandGroup>
    </>
  )
}
