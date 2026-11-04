import { GenericFormField } from '@md/ui'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

import { Autocomplete } from '@/components/form/Autocomplete'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { discoverEventCodes } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'

const EventCodeInput = ({
  value,
  onChange,
  placeholder,
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
}) => {
  // Server-search the distinct codes as the user types (debounced); an empty query
  // returns the most-frequent codes, so focusing the field surfaces recent sources.
  const search = useDebounceValue(value.trim(), 250)
  const { data, isLoading } = useQuery(discoverEventCodes, { search, limit: 50 })

  const items = (data?.codes ?? []).map(c => ({
    value: c.code,
    node: (
      <>
        <span className="truncate font-mono">{c.code}</span>
        <span className="ml-auto shrink-0 pl-2 text-[10px] text-muted-foreground">
          {Number(c.count).toLocaleString()} events
        </span>
      </>
    ),
  }))

  return (
    <Autocomplete
      value={value}
      onValueChange={onChange}
      items={items}
      isLoading={isLoading}
      placeholder={placeholder}
      emptyText="No matching event codes. Type to use a new one."
    />
  )
}

interface Props<T extends FieldValues> {
  name: FieldPath<T>
  control: Control<T>
  label?: string
  placeholder?: string
  description?: string
}

/**
 * The event-code field bound to react-hook-form, autocompleting the tenant's distinct
 * event codes (most-frequent first) while allowing a new, not-yet-seen code.
 */
export const EventCodeFormField = <T extends FieldValues>({
  name,
  control,
  label,
  placeholder,
  description,
}: Props<T>) => (
  <GenericFormField
    name={name}
    control={control}
    label={label}
    description={description}
    render={({ field }) => (
      <EventCodeInput
        value={(field.value as string) ?? ''}
        onChange={v => field.onChange(v)}
        placeholder={placeholder}
      />
    )}
  />
)
