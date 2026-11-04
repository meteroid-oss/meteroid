import { Badge, Input, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { cn } from '@ui/lib'

import { OPTION_WIDGETS, OptionPicker, OptionWidget } from './OptionPicker'
import { FilterOp, MULTI_VALUE_OPS, VALUELESS_OPS } from './filterTypes'
import { ResolvedField } from './useFilterFields'

interface Props {
  field: ResolvedField
  op: FilterOp
  values: string[]
  onChange: (values: string[]) => void
  autoFocus?: boolean
}

const inputType = (widget: ResolvedField['widget']): string => {
  switch (widget) {
    case 'number':
    case 'money':
      return 'number'
    case 'date':
      return 'date'
    case 'datetime':
      return 'datetime-local'
    default:
      return 'text'
  }
}

/** Type-aware value editor for a single filter. Renders nothing for valueless operators. */
export const FilterValueInput = ({ field, op, values, onChange, autoFocus }: Props) => {
  if (VALUELESS_OPS.has(op)) return null

  // List-backed values (country, currency, plan, customer): searchable picker.
  if (OPTION_WIDGETS.has(field.widget)) {
    return (
      <OptionPicker
        widget={field.widget as OptionWidget}
        values={values}
        multi={op === 'in'}
        onChange={onChange}
      />
    )
  }

  const isBetween = op === 'between'
  const isMulti = MULTI_VALUE_OPS.has(op)

  // Multi-select over defined options (SINGLE_SELECT / MULTI_SELECT with `in`).
  if (field.widget === 'select' && (op === 'in' || op === 'contains')) {
    const selected = new Set(values)
    const toggle = (v: string) =>
      onChange(selected.has(v) ? values.filter(x => x !== v) : [...values, v])
    return (
      <div className="flex flex-wrap gap-1.5">
        {(field.options ?? []).map(o => (
          <Badge
            key={o.value}
            variant={selected.has(o.value) ? 'default' : 'outline'}
            className="cursor-pointer"
            onClick={() => toggle(o.value)}
          >
            {o.label}
          </Badge>
        ))}
      </div>
    )
  }

  // Single-select over defined options.
  if (field.widget === 'select') {
    return (
      <Select value={values[0] ?? ''} onValueChange={v => onChange([v])}>
        <SelectTrigger className="h-8">
          <SelectValue placeholder="Select…" />
        </SelectTrigger>
        <SelectContent>
          {(field.options ?? []).map(o => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    )
  }

  // Range: two bounds.
  if (isBetween) {
    return (
      <div className="flex items-center gap-2">
        <Input
          type={inputType(field.widget)}
          className="h-8"
          autoFocus={autoFocus}
          value={values[0] ?? ''}
          onChange={e => onChange([e.target.value, values[1] ?? ''])}
        />
        <span className="text-muted-foreground text-xs">and</span>
        <Input
          type={inputType(field.widget)}
          className="h-8"
          value={values[1] ?? ''}
          onChange={e => onChange([values[0] ?? '', e.target.value])}
        />
      </div>
    )
  }

  // `in` over free text/number: comma-separated tokens.
  if (isMulti) {
    return (
      <Input
        className={cn('h-8')}
        autoFocus={autoFocus}
        placeholder="value, value, …"
        value={values.join(', ')}
        onChange={e =>
          onChange(
            e.target.value
              .split(',')
              .map(s => s.trim())
              .filter(Boolean)
          )
        }
      />
    )
  }

  // Single value.
  return (
    <Input
      type={inputType(field.widget)}
      className="h-8"
      autoFocus={autoFocus}
      value={values[0] ?? ''}
      onChange={e => onChange(e.target.value === '' ? [] : [e.target.value])}
    />
  )
}
