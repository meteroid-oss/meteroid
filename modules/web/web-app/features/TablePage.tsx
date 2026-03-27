import { SearchIcon } from '@md/icons'
import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  InputWithIcon,
} from '@ui/components'
import { cn } from '@ui/lib'
import { ChevronDown, PlusIcon, RefreshCwIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import { SetQueryStateAction, useQueryState } from '@/hooks/useQueryState'

interface Action {
  label: string
  icon?: React.ReactNode
  onClick: () => void
}

type EntityHeaderProps = {
  title: string
  count?: number
  primaryAction?: Action
  secondaryActions?: Action[]
}

export const EntityHeader = ({
  title,
  count,
  primaryAction,
  secondaryActions,
}: EntityHeaderProps) => {
  const hasSecondaryActions = !!secondaryActions?.length
  return (
    <div className="flex justify-between items-center">
      <h1 className="text-2xl font-bold">
        {title}{' '}
        {count !== undefined && (
          <span className="text-xs font-medium text-muted-foreground">({count})</span>
        )}
      </h1>
      {primaryAction && (
        <div className="flex gap-0.5">
          <Button
            variant="primary"
            size="sm"
            onClick={primaryAction.onClick}
            hasIcon
            className={cn(hasSecondaryActions && 'border-r-0 rounded-r-none')}
          >
            {primaryAction.icon ?? <PlusIcon className="w-4 h-4" />} {primaryAction.label}
          </Button>
          {hasSecondaryActions && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="primary"
                  className="gap-2 border-l-0  rounded-l-none"
                  size="sm"
                  hasIcon
                >
                  <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {secondaryActions.map((option, optionIndex) => (
                  <DropdownMenuItem key={optionIndex} onClick={option.onClick}>
                    {option.label}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      )}
    </div>
  )
}

interface EntityFiltersProps {
  className?: string
  children?: React.ReactNode
  isLoading?: boolean
  refetch?: () => void
}

export const EntityFilters = ({ children, className, isLoading, refetch }: EntityFiltersProps) => {
  const [search, setSearch] = useQueryState<string>('q', '') // Changed from undefined to empty string

  return (
    <div className={cn('flex flex-row items-center justify-between gap-2', className)}>
      <div className="flex flex-row items-center gap-2">
        <InputWithIcon
          placeholder="Search..."
          icon={<SearchIcon size={16} />}
          width="fit-content"
          onChange={e => setSearch(e.target.value)}
          value={search ?? ''}
        />
        {children && (
          <div className={cn('flex gap-2 items-center')}>{children}</div>
        )}
      </div>
      {refetch && (
        <Button variant="outline" size="sm" disabled={isLoading} onClick={refetch}>
          <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
        </Button>
      )}
    </div>
  )
}

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
