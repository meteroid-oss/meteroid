import { GenericFormField } from '@md/ui'
import { AlertTriangleIcon } from 'lucide-react'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

import { Autocomplete, type AutocompleteItem } from '@/components/form/Autocomplete'
import { useQuery } from '@/lib/connectrpc'
import { discoverProperties } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'

/** Above this fraction of numeric values a property is treated as a number. */
const NUMERIC_THRESHOLD = 0.8

const TypeBadge = ({ numeric }: { numeric: boolean }) => (
  <span className="inline-flex shrink-0 items-center rounded bg-muted px-1 py-0.5 text-[9px] font-medium uppercase tracking-wide text-muted-foreground">
    {numeric ? 'num' : 'text'}
  </span>
)

export const PropertyKeyInput = ({
  eventCode,
  value,
  onChange,
  placeholder,
  numericExpected,
}: {
  eventCode: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  numericExpected?: boolean
}) => {
  const { data, isLoading } = useQuery(
    discoverProperties,
    { eventCode, lookbackDays: 90, limit: 100 },
    { enabled: eventCode.trim().length > 0 }
  )
  const properties = data?.properties ?? []

  const items: AutocompleteItem[] = properties.map(p => {
    const numeric = p.numericRatio >= NUMERIC_THRESHOLD
    return {
      value: p.key,
      keywords: p.sampleValues,
      node: (
        <>
          <TypeBadge numeric={numeric} />
          <span className="truncate font-mono">{p.key}</span>
          <span className="ml-auto shrink-0 pl-2 text-[10px] text-muted-foreground">
            {Number(p.cardinality).toLocaleString()} values
          </span>
        </>
      ),
    }
  })

  // Soft guidance: warn (never block) when a numeric aggregation targets a property
  // whose observed values are mostly non-numeric.
  const selected = properties.find(p => p.key === value.trim())
  const mismatch = !!numericExpected && !!selected && selected.numericRatio < NUMERIC_THRESHOLD

  return (
    <div>
      <Autocomplete
        value={value}
        onValueChange={onChange}
        items={items}
        isLoading={isLoading}
        placeholder={placeholder}
        emptyText={
          eventCode.trim().length === 0
            ? 'Set the event code first to discover its properties.'
            : 'No matching properties in recent events. Type to use a custom key.'
        }
      />
      {mismatch && (
        <p className="mt-1 flex items-center gap-1 text-[11px] text-amber-600 dark:text-amber-500">
          <AlertTriangleIcon className="h-3 w-3 shrink-0" />“{value.trim()}” looks like text — this
          aggregation expects a numeric property.
        </p>
      )}
    </div>
  )
}

interface Props<T extends FieldValues> {
  name: FieldPath<T>
  control: Control<T>
  /** Event code the keys are discovered from; empty disables discovery (free text still works). */
  eventCode: string
  label?: string
  placeholder?: string
  description?: string
  /** When true, softly warn if the chosen property is not numeric (SUM/MIN/MAX/MEAN). */
  numericExpected?: boolean
}

/**
 * A metric-builder key field bound to react-hook-form that autocompletes the event
 * source's actual property keys (scoped to the selected event code), tags each as
 * number/text, and softly warns on an obvious type mismatch. Free-typed keys are
 * always allowed — a late-arriving/low-volume key may not be discovered yet.
 */
export const PropertyKeyFormField = <T extends FieldValues>({
  name,
  control,
  eventCode,
  label,
  placeholder,
  description,
  numericExpected,
}: Props<T>) => (
  <GenericFormField
    name={name}
    control={control}
    label={label}
    description={description}
    render={({ field }) => (
      <PropertyKeyInput
        eventCode={eventCode}
        value={(field.value as string) ?? ''}
        onChange={v => field.onChange(v)}
        placeholder={placeholder}
        numericExpected={numericExpected}
      />
    )}
  />
)
