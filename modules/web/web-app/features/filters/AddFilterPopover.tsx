import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@md/ui'
import { ListFilterIcon } from 'lucide-react'
import { useState } from 'react'

import { FilterEditor } from './FilterEditor'
import { AppFilter, FilterOp, VALUELESS_OPS, carryValues } from './filterTypes'
import { ResolvedField } from './useFilterFields'

interface Props {
  fields: ResolvedField[]
  onAdd: (filter: AppFilter) => void
}

/** "＋ Filter" entry point: pick a property, then an operator + value, then add. */
export const AddFilterPopover = ({ fields, onAdd }: Props) => {
  const [open, setOpen] = useState(false)
  const [picked, setPicked] = useState<ResolvedField | null>(null)
  const [op, setOp] = useState<FilterOp>('equals')
  const [values, setValues] = useState<string[]>([])

  const reset = () => {
    setPicked(null)
    setOp('equals')
    setValues([])
  }

  const choose = (field: ResolvedField) => {
    setPicked(field)
    setOp(field.ops[0])
    setValues([])
  }

  const canApply = picked && (VALUELESS_OPS.has(op) || values.length > 0)

  const apply = () => {
    if (!picked) return
    onAdd({ field: picked.field, op, values })
    reset()
    setOpen(false)
  }

  // In the order the fields come: a page's own fields first, then the entity's.
  const groups = Array.from(new Set(fields.map(f => f.group)))

  return (
    <Popover
      open={open}
      onOpenChange={o => {
        setOpen(o)
        if (!o) reset()
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="outline" className="text-xs font-medium" hasIcon>
          <ListFilterIcon size="14" />
          <span>Filter</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0">
        {!picked ? (
          <Command>
            <CommandInput placeholder="Filter by…" />
            <CommandList>
              <CommandEmpty>No property found.</CommandEmpty>
              {groups.map(group => {
                const groupFields = fields.filter(f => f.group === group)
                if (groupFields.length === 0) return null
                return (
                  <CommandGroup key={group} heading={group}>
                    {groupFields.map(f => (
                      <CommandItem key={f.field} value={f.label} onSelect={() => choose(f)}>
                        {f.label}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                )
              })}
            </CommandList>
          </Command>
        ) : (
          <div className="flex flex-col gap-3 p-3">
            <div className="text-sm font-medium">{picked.label}</div>
            <FilterEditor
              field={picked}
              op={op}
              values={values}
              onOpChange={o => {
                setValues(v => carryValues(op, o, v))
                setOp(o)
              }}
              onValuesChange={setValues}
            />
            <div className="flex justify-between gap-2">
              <Button variant="ghost" size="sm" onClick={reset}>
                Back
              </Button>
              <Button size="sm" disabled={!canApply} onClick={apply}>
                Add filter
              </Button>
            </div>
          </div>
        )}
      </PopoverContent>
    </Popover>
  )
}
