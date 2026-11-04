import { useState } from 'react'

import {
  Autocomplete,
  AutocompleteTags,
  type AutocompleteItem,
} from '@/components/form/Autocomplete'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { discoverPropertyValues } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'

const valueItem = (value: string, count: bigint): AutocompleteItem => ({
  value,
  node: (
    <>
      <span className="truncate">{value}</span>
      <span className="ml-auto shrink-0 pl-2 text-[10px] text-muted-foreground">
        {Number(count).toLocaleString()}
      </span>
    </>
  ),
})

const useDiscoveredValues = (eventCode: string, propertyKey: string, query: string) => {
  const search = useDebounceValue(query.trim(), 250)
  const enabled = eventCode.trim().length > 0 && propertyKey.trim().length > 0
  const { data, isLoading } = useQuery(
    discoverPropertyValues,
    { eventCode, propertyKey, search, limit: 20 },
    { enabled }
  )
  const items = (data?.values ?? []).map(v => valueItem(v.value, v.count))
  return { items, isLoading, enabled }
}

const emptyText = (enabled: boolean): string =>
  enabled ? 'No matching values in recent events.' : 'Choose a property first to suggest values.'

/** Single-value property-value field (EQUAL / NOT_EQUAL filters), autocompleting the
 *  property's observed values for the selected event code. */
export const PropertyValueInput = ({
  eventCode,
  propertyKey,
  value,
  onChange,
  placeholder,
  className,
}: {
  eventCode: string
  propertyKey: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  className?: string
}) => {
  const { items, isLoading, enabled } = useDiscoveredValues(eventCode, propertyKey, value)
  return (
    <Autocomplete
      value={value}
      onValueChange={onChange}
      items={items}
      isLoading={isLoading}
      placeholder={placeholder}
      emptyText={emptyText(enabled)}
      className={className}
    />
  )
}

/** Multi-value property-value tags (IN / NOT_IN filters, segmentation dimension
 *  values), autocompleting observed values as chips. */
export const PropertyValueTags = ({
  eventCode,
  propertyKey,
  value,
  onChange,
  placeholder,
}: {
  eventCode: string
  propertyKey: string
  value: string[] | undefined
  onChange: (v: string[]) => void
  placeholder?: string
}) => {
  const [query, setQuery] = useState('')
  const { items, isLoading, enabled } = useDiscoveredValues(eventCode, propertyKey, query)
  return (
    <AutocompleteTags
      value={value}
      onChange={onChange}
      items={items}
      onQueryChange={setQuery}
      isLoading={isLoading}
      placeholder={placeholder}
      emptyText={emptyText(enabled)}
    />
  )
}
