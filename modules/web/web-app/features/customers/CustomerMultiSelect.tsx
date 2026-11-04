import {
  MultiSelect,
  MultiSelectContent,
  MultiSelectEmpty,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectSearch,
  MultiSelectTrigger,
  MultiSelectValue,
} from '@md/ui'
import { useMemo, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  listCustomers,
  listCustomersByIds,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'

interface Props {
  value: string[]
  onChange: (ids: string[]) => void
  placeholder?: string
  className?: string
}

/**
 * Searchable multi-customer picker backed by server-side `listCustomers` search.
 * Used to scope reports to one or more customers. Selected ids that fall outside
 * the current search page keep their names (batch-hydrated in one request) so the
 * selection is always legible.
 */
export const CustomerMultiSelect = ({ value, onChange, placeholder, className }: Props) => {
  const [search, setSearch] = useState<string | undefined>(undefined)

  const list = useQuery(listCustomers, {
    pagination: { perPage: 20, page: 0 },
    search: search && search.length > 0 ? search : undefined,
  })
  const results = useMemo(() => list.data?.customers ?? [], [list.data?.customers])

  // Hydrate names for the whole selection in one batched request, keyed on the
  // stable (sorted) selection so it doesn't refire as search pages resolve names.
  const selectedIds = useMemo(() => [...value].sort(), [value])
  const hydrate = useQuery(
    listCustomersByIds,
    { ids: selectedIds },
    { enabled: selectedIds.length > 0 }
  )

  // Names read fresh from the search page + hydration each render — no cache to go
  // stale, so a renamed customer reflects immediately.
  const labels = useMemo(() => {
    const map: Record<string, string> = {}
    for (const c of hydrate.data?.customers ?? []) map[c.id] = c.name
    for (const c of results) map[c.id] = c.name
    return map
  }, [results, hydrate.data?.customers])

  // Every result, plus any selected customer off the current page (Set membership),
  // so its checked row stays visible while the search narrows the list.
  const optionIds = useMemo(() => {
    const resultIds = results.map(c => c.id)
    const seen = new Set(resultIds)
    return [...resultIds, ...value.filter(id => !seen.has(id))]
  }, [results, value])

  return (
    <MultiSelect value={value} onValueChange={onChange} onSearch={setSearch}>
      <MultiSelectTrigger className={className}>
        <MultiSelectValue
          placeholder={placeholder ?? 'All customers'}
          maxDisplay={2}
          maxItemLength={18}
        />
      </MultiSelectTrigger>
      <MultiSelectContent>
        <MultiSelectSearch placeholder="Search customers…" />
        <MultiSelectList>
          <MultiSelectEmpty>No customers found</MultiSelectEmpty>
          {optionIds.map(id => (
            <MultiSelectItem key={id} value={id} label={labels[id] ?? id}>
              {labels[id] ?? id}
            </MultiSelectItem>
          ))}
        </MultiSelectList>
      </MultiSelectContent>
    </MultiSelect>
  )
}
