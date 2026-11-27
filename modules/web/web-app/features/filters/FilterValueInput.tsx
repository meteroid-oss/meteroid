import { Input, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { cn } from '@ui/lib'
import { Check, Minus } from 'lucide-react'

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
    // A checklist, as the searchable pickers show theirs; grouped options get a row per group
    // that toggles the whole group.
    const options = field.options ?? []
    const groups = Array.from(new Set(options.map(o => o.group)))
    const setGroup = (members: string[], on: boolean) =>
      onChange(
        on
          ? [...values, ...members.filter(v => !selected.has(v))]
          : values.filter(v => !members.includes(v))
      )
    return (
      <div role="group" aria-label={field.label} className="-mx-1 flex flex-col gap-px">
        {groups.map(group => {
          const members = options.filter(o => o.group === group)
          const count = members.filter(o => selected.has(o.value)).length
          return (
            <div key={group ?? ''} className="flex flex-col gap-px">
              {group && (
                <CheckRow
                  label={group}
                  checked={count === 0 ? false : count === members.length ? true : 'mixed'}
                  onToggle={() =>
                    setGroup(
                      members.map(o => o.value),
                      count < members.length
                    )
                  }
                  strong
                />
              )}
              {members.map(o => (
                <CheckRow
                  key={o.value}
                  label={o.label}
                  checked={selected.has(o.value)}
                  onToggle={() => toggle(o.value)}
                  inset={Boolean(group)}
                />
              ))}
            </div>
          )
        })}
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

/** One option of a checklist: a small box (ticked, dashed for a part of a group) and its label. */
const CheckRow = ({
  label,
  checked,
  onToggle,
  inset = false,
  strong = false,
}: {
  label: string
  checked: boolean | 'mixed'
  onToggle: () => void
  inset?: boolean
  strong?: boolean
}) => (
  <button
    type="button"
    role="checkbox"
    aria-checked={checked}
    onClick={onToggle}
    className={cn(
      'flex h-[30px] items-center gap-2 rounded-[5px] px-2 text-left text-[13px] transition-colors hover:bg-foreground/[0.06] focus-visible:outline-none focus-visible:bg-foreground/[0.08]',
      inset && 'pl-7',
      strong ? 'font-medium text-foreground' : 'text-foreground/90'
    )}
  >
    <span
      aria-hidden
      className={cn(
        'flex size-3.5 shrink-0 items-center justify-center rounded-[3px] border-[0.5px]',
        checked ? 'border-foreground bg-foreground text-background' : 'border-foreground/30'
      )}
    >
      {checked === true && <Check className="size-2.5" strokeWidth={3} />}
      {checked === 'mixed' && <Minus className="size-2.5" strokeWidth={3} />}
    </span>
    {label}
  </button>
)
