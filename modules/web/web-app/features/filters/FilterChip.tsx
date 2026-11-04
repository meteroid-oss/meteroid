import { Popover, PopoverContent, PopoverTrigger } from '@md/ui'
import { AlertTriangleIcon, XIcon } from 'lucide-react'
import { useState } from 'react'

import { FilterEditor } from './FilterEditor'
import { OPTION_WIDGETS, OptionSummary, OptionWidget } from './OptionPicker'
import { AppFilter, OP_LABELS, VALUELESS_OPS, carryValues } from './filterTypes'
import { ResolvedField } from './useFilterFields'

interface Props {
  filter: AppFilter
  field?: ResolvedField
  /** Whether field definitions have loaded (an unknown field is only "unavailable" once they have). */
  loaded?: boolean
  onChange: (filter: AppFilter) => void
  onRemove: () => void
}

/** Summarise the selected values for the chip label. */
function valueSummary(filter: AppFilter, field?: ResolvedField): string {
  if (VALUELESS_OPS.has(filter.op)) return ''
  const labelOf = (v: string) => field?.options?.find(o => o.value === v)?.label ?? v
  if (filter.op === 'between') return filter.values.map(labelOf).join(' – ')
  return filter.values.map(labelOf).join(', ')
}

/** A single active filter, shown as an editable + removable chip. */
export const FilterChip = ({ filter, field, loaded, onChange, onRemove }: Props) => {
  const [open, setOpen] = useState(false)
  const label = field?.label ?? filter.field.replace(/^cp:/, '')
  const summary = valueSummary(filter, field)
  // Field removed (e.g. archived custom property) or operator no longer allowed: the filter is
  // skipped by the request (see useRequestFilters) — show why, and let the user remove it.
  const unavailable = !!loaded && (!field || !field.ops.includes(filter.op))

  if (unavailable) {
    return (
      <div
        className="flex items-center rounded-md border border-dashed text-xs text-muted-foreground"
        title="This field is no longer available, so this filter is ignored"
      >
        <span className="flex items-center gap-1 px-2 py-1">
          <AlertTriangleIcon size="12" />
          <span className="line-through">{label}</span>
        </span>
        <button
          type="button"
          className="px-1.5 py-1 hover:text-foreground"
          onClick={onRemove}
          aria-label="Remove filter"
        >
          <XIcon size="12" />
        </button>
      </div>
    )
  }

  return (
    <div className="flex items-center rounded-md border bg-muted/40 text-xs">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <button className="flex items-center gap-1 px-2 py-1 hover:bg-muted" type="button">
            <span className="font-medium">{label}</span>
            <span className="text-muted-foreground">{OP_LABELS[filter.op]}</span>
            {field && OPTION_WIDGETS.has(field.widget) && !VALUELESS_OPS.has(filter.op) ? (
              <span className="font-medium">
                <OptionSummary widget={field.widget as OptionWidget} values={filter.values} />
              </span>
            ) : (
              summary && <span className="font-medium">{summary}</span>
            )}
          </button>
        </PopoverTrigger>
        {field && (
          <PopoverContent align="start" className="w-72 p-3">
            <FilterEditor
              field={field}
              op={filter.op}
              values={filter.values}
              onOpChange={op =>
                onChange({ ...filter, op, values: carryValues(filter.op, op, filter.values) })
              }
              onValuesChange={values => onChange({ ...filter, values })}
            />
          </PopoverContent>
        )}
      </Popover>
      <button
        type="button"
        className="px-1.5 py-1 text-muted-foreground hover:text-foreground"
        onClick={onRemove}
        aria-label="Remove filter"
      >
        <XIcon size="12" />
      </button>
    </div>
  )
}
