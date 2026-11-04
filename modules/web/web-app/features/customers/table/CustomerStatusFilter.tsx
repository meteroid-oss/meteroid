import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@md/ui'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent } from 'react'

import { LIFECYCLE_BUCKETS, LifecycleState, lifecycleStateLabel } from '@/lib/lifecycle'

/** Archived scope, orthogonal to lifecycle. Maps to the list request `archived`
 * field: active -> false, archived -> true, all -> undefined. */
export type ArchivedScope = 'active' | 'archived' | 'all'

interface Props {
  archived: ArchivedScope
  onArchivedChange: (scope: ArchivedScope) => void
  /** Selected lifecycle states (union). Empty = no lifecycle filter. */
  lifecycle: string[]
  onLifecycleChange: (states: string[]) => void
}

/** One grouped dropdown: a single-select "Records" scope (archived) on top, then
 * the lifecycle buckets whose headers toggle the whole bucket and whose items are
 * the individual states. Both write one canonical lifecycle_state[] array. */
export const CustomerStatusFilter: FunctionComponent<Props> = ({
  archived,
  onArchivedChange,
  lifecycle,
  onLifecycleChange,
}) => {
  const selected = new Set(lifecycle)

  const toggleState = (state: LifecycleState) => {
    const next = new Set(selected)
    if (next.has(state)) next.delete(state)
    else next.add(state)
    onLifecycleChange([...next])
  }

  const toggleBucket = (states: LifecycleState[]) => {
    const allSelected = states.every(s => selected.has(s))
    const next = new Set(selected)
    if (allSelected) states.forEach(s => next.delete(s))
    else states.forEach(s => next.add(s))
    onLifecycleChange([...next])
  }

  const bucketChecked = (states: LifecycleState[]): boolean | 'indeterminate' => {
    const count = states.filter(s => selected.has(s)).length
    if (count === 0) return false
    if (count === states.length) return true
    return 'indeterminate'
  }

  const parts: string[] = []
  if (archived === 'archived') parts.push('Archived')
  else if (archived === 'all') parts.push('All records')
  if (lifecycle.length === 1) parts.push(lifecycleStateLabel(lifecycle[0]))
  else if (lifecycle.length > 1) parts.push(`${lifecycle.length} lifecycles`)
  const triggerLabel = parts.length > 0 ? parts.join(' · ') : 'Status'

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" className="text-xs font-medium" hasIcon>
          <span>{triggerLabel}</span>
          <ChevronDown size="14" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-60">
        <DropdownMenuLabel className="text-[11px] uppercase tracking-wide text-muted-foreground">
          Records
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={archived}
          onValueChange={v => onArchivedChange(v as ArchivedScope)}
        >
          <DropdownMenuRadioItem value="active">Active (not archived)</DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="archived">Archived</DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="all">All</DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>

        <DropdownMenuSeparator />

        <div className="flex items-center justify-between px-2 py-1.5">
          <span className="text-[11px] uppercase tracking-wide text-muted-foreground">
            Lifecycle
          </span>
          {lifecycle.length > 0 && (
            <button
              className="text-[11px] text-muted-foreground hover:text-foreground"
              onClick={() => onLifecycleChange([])}
            >
              Clear
            </button>
          )}
        </div>
        {LIFECYCLE_BUCKETS.map(bucket => (
          <div key={bucket.key} className="mb-1">
            <DropdownMenuCheckboxItem
              checked={bucketChecked(bucket.states)}
              onCheckedChange={() => toggleBucket(bucket.states)}
              onSelect={e => e.preventDefault()}
              className="font-medium"
            >
              {bucket.label}
            </DropdownMenuCheckboxItem>
            {bucket.states.map(state => (
              <DropdownMenuCheckboxItem
                key={state}
                checked={selected.has(state)}
                onCheckedChange={() => toggleState(state)}
                onSelect={e => e.preventDefault()}
                className="pl-8 text-muted-foreground"
              >
                {lifecycleStateLabel(state)}
              </DropdownMenuCheckboxItem>
            ))}
          </div>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
