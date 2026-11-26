import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from '@md/ui'
import { CheckIcon, ChevronsUpDownIcon } from 'lucide-react'
import { type ReactNode, useState } from 'react'

import { type Target } from '@/features/entitlements/explain/checkRequest'
import { humanize, keyLabel, lower, useCustomerResources, useResourceTypes } from '@/features/resources/hooks'

const triggerLabel = (t: Target) =>
  t.kind === 'customer'
    ? 'Whole customer'
    : t.kind === 'stored'
      ? `${t.type.name} · ${t.resource.name || t.resource.keyValue}`
      : `New ${lower(t.type.name)}`

export const EvaluateFor = ({
  customerId,
  customerName,
  value,
  onChange,
}: {
  customerId: string
  customerName: string
  value: Target
  onChange: (t: Target) => void
}) => {
  const { types } = useResourceTypes()
  const { resources } = useCustomerResources(customerId)
  const [open, setOpen] = useState(false)

  const pick = (t: Target) => {
    onChange(t)
    setOpen(false)
  }
  const isSelected = (t: Target) =>
    t.kind === value.kind &&
    (t.kind === 'customer' ||
      (t.kind === 'stored' && value.kind === 'stored' && t.resource.id === value.resource.id) ||
      (t.kind === 'adhoc' && value.kind === 'adhoc' && t.type.id === value.type.id))

  if (!types.length) return null
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            size="sm"
            role="combobox"
            aria-expanded={open}
            aria-label="Evaluate for"
            className="mt-0 h-8 min-w-56 justify-between gap-2 px-3 text-[13px] font-normal"
          >
            <span className="truncate">{triggerLabel(value)}</span>
            <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-72 p-0">
          <Command>
            <CommandInput placeholder="Search" className="h-9 text-[13px]" />
            <CommandList>
              <CommandEmpty>No match</CommandEmpty>
              <CommandGroup>
                <Option
                  value="customer"
                  keywords={[customerName, 'whole customer']}
                  selected={value.kind === 'customer'}
                  onSelect={() => pick({ kind: 'customer' })}
                >
                  <span className="truncate">Whole customer</span>
                  <span className="truncate text-xs text-muted-foreground">{customerName}</span>
                </Option>
              </CommandGroup>
              {types.map(type => {
                const adhoc: Target = { kind: 'adhoc', type, keyValue: '', attributes: {} }
                return (
                  <CommandGroup key={type.id} heading={type.pluralName}>
                    {resources
                      .filter(r => r.resourceTypeId === type.id)
                      .map(resource => {
                        const t: Target = { kind: 'stored', type, resource }
                        return (
                          <Option
                            key={resource.id}
                            value={`stored:${resource.id}`}
                            keywords={[resource.name ?? '', resource.keyValue, type.name]}
                            selected={isSelected(t)}
                            onSelect={() => pick(t)}
                          >
                            <span className="truncate">{resource.name || resource.keyValue}</span>
                            {resource.name && (
                              <span className="truncate font-mono text-xs text-muted-foreground">{resource.keyValue}</span>
                            )}
                          </Option>
                        )
                      })}
                    <Option
                      value={`adhoc:${type.id}`}
                      keywords={[`new ${type.name}`]}
                      selected={isSelected(adhoc)}
                      onSelect={() => pick(value.kind === 'adhoc' && value.type.id === type.id ? value : adhoc)}
                    >
                      <span className="truncate text-muted-foreground">New {lower(type.name)}…</span>
                    </Option>
                  </CommandGroup>
                )
              })}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
      {value.kind === 'adhoc' && (
        <>
          <Input
            className="h-8 w-36 text-[13px]"
            placeholder={keyLabel(value.type)}
            aria-label={keyLabel(value.type)}
            value={value.keyValue}
            onChange={e => onChange({ ...value, keyValue: e.target.value })}
          />
          {Object.entries(value.type.attributes).map(([attribute, { values }]) => (
            <Select
              key={attribute}
              value={value.attributes[attribute] ?? ''}
              onValueChange={v => onChange({ ...value, attributes: { ...value.attributes, [attribute]: v } })}
            >
              <SelectTrigger className="h-8 w-40 text-[13px]" aria-label={humanize(attribute)}>
                <SelectValue placeholder={humanize(attribute)} />
              </SelectTrigger>
              <SelectContent>
                {values.map(v => (
                  <SelectItem key={v} value={v}>
                    {humanize(attribute)}: {v}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ))}
        </>
      )}
    </div>
  )
}

const Option = ({
  value,
  keywords,
  selected,
  onSelect,
  children,
}: {
  value: string
  keywords: string[]
  selected: boolean
  onSelect: () => void
  children: ReactNode
}) => (
  <CommandItem value={value} keywords={keywords.filter(Boolean)} onSelect={onSelect} className="gap-2 text-[13px]">
    <span className="flex min-w-0 flex-1 items-baseline gap-2">{children}</span>
    <CheckIcon className={cn('size-3.5 shrink-0', selected ? 'opacity-100' : 'opacity-0')} />
  </CommandItem>
)
