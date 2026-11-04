import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent } from 'react'

import { SetQueryStateAction } from '@/hooks/useQueryState'

type Hook<A> = [A, (value: SetQueryStateAction<A>) => void]
type FilterState<A> = {
  hook: Hook<A>
  emptyLabel: string
  entries: Array<string | { label: string; value: string }>
}

export const MultiFilter: FunctionComponent<FilterState<string[]>> = ({
  entries,
  emptyLabel,
  hook,
}) => {
  const [state, setState] = hook

  const handleSelectionChange = (value: string, checked: boolean) => {
    if (checked) {
      setState(state => [...state, value])
    } else {
      setState(state => state.filter(item => item !== value))
    }
  }

  return (
    <BaseFilter
      entries={entries}
      emptyLabel={emptyLabel}
      selected={state}
      onSelectionChange={handleSelectionChange}
    />
  )
}

export const SingleFilter: FunctionComponent<FilterState<string | undefined>> = ({
  entries,
  emptyLabel,
  hook,
}) => {
  const [state, setState] = hook

  const handleSelectionChange = (value: string, checked: boolean) => {
    setState(checked ? value : undefined)
  }

  return (
    <BaseFilter
      entries={entries}
      emptyLabel={emptyLabel}
      selected={state ? [state] : []}
      onSelectionChange={handleSelectionChange}
    />
  )
}

interface BaseFilterProps {
  emptyLabel: string
  entries: Array<string | { label: string; value: string }>
  selected: string[]
  onSelectionChange: (value: string, checked: boolean) => void
}

export const BaseFilter: FunctionComponent<BaseFilterProps> = ({
  entries,
  emptyLabel,
  selected,
  onSelectionChange,
}) => {
  const getLabel = (value: string) => {
    const entry = entries.find(e => (typeof e === 'string' ? e : e.value) === value)
    return entry ? (typeof entry === 'string' ? entry : entry.label) : value
  }

  const triggerLabel =
    selected.length > 0 ? selected.map(getLabel).join(', ') : emptyLabel

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" className="text-xs font-medium" hasIcon>
          <span className="capitalize">{triggerLabel}</span>
          <ChevronDown size="14" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {entries.map(entry =>
          typeof entry === 'string' ? (
            <DropdownMenuCheckboxItem
              key={entry}
              className="capitalize"
              checked={selected.includes(entry)}
              onCheckedChange={checked => onSelectionChange(entry, checked)}
            >
              {entry}
            </DropdownMenuCheckboxItem>
          ) : (
            <DropdownMenuCheckboxItem
              key={entry.value}
              className="capitalize"
              checked={selected.includes(entry.value)}
              onCheckedChange={checked => onSelectionChange(entry.value, checked)}
            >
              {entry.label}
            </DropdownMenuCheckboxItem>
          )
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
