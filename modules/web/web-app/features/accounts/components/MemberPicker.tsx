import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from '@md/ui'
import { Check, ChevronsUpDown } from 'lucide-react'
import { useState } from 'react'

import { MemberAvatar } from '@/features/accounts/components/MemberAvatar'
import { useMembers } from '@/features/accounts/hooks/useMembers'

/**
 * Picks an organization member, e.g. an account owner. `onChange(undefined)` unassigns.
 * Read-only when `disabled`: the current member still shows.
 */
export const MemberPicker = ({
  value,
  onChange,
  label,
  placeholder = 'Unassigned',
  disabled,
  className,
  align = 'start',
  compact = false,
  field = false,
}: {
  value?: string
  onChange: (userId: string | undefined) => void
  /** Accessible name of the control, e.g. "Account owner". */
  label: string
  placeholder?: string
  disabled?: boolean
  className?: string
  align?: 'start' | 'end'
  /** The avatar alone (an empty circle when nobody), for tight rows. */
  compact?: boolean
  /** A form field beside selects: their height, background and border. */
  field?: boolean
}) => {
  const [open, setOpen] = useState(false)
  const { members, meId, isLoading } = useMembers()

  const pick = (userId: string | undefined) => {
    setOpen(false)
    if (userId !== value) onChange(userId)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild disabled={disabled}>
        <Button
          variant="ghost"
          size="sm"
          role="combobox"
          aria-expanded={open}
          aria-label={label}
          disabled={disabled}
          className={cn(
            compact
              ? 'h-6 w-6 rounded-full p-0'
              : field
                ? 'h-9 w-full justify-between gap-2 rounded-md border border-border bg-input px-3 py-2 text-sm font-normal shadow-sm hover:bg-input focus:ring-1 focus:ring-ring'
                : 'h-8 justify-between gap-2 px-2 font-normal',
            className
          )}
        >
          {compact ? (
            value ? (
              <MemberAvatar userId={value} size="sm" />
            ) : (
              <span
                aria-hidden
                className="h-5 w-5 rounded-full border border-dashed border-muted-foreground/50"
              />
            )
          ) : value ? (
            <MemberAvatar userId={value} showName size={field ? 'sm' : 'md'} />
          ) : (
            <span className="text-muted-foreground">{placeholder}</span>
          )}
          {!disabled && !compact && (
            <ChevronsUpDown
              className={cn(
                'shrink-0',
                field ? 'h-4 w-4 opacity-50' : 'h-3.5 w-3.5 text-muted-foreground'
              )}
            />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-0" align={align}>
        <Command>
          <CommandInput placeholder="Search members…" />
          <CommandList>
            <CommandEmpty>{isLoading ? 'Loading members…' : 'No member found'}</CommandEmpty>
            {meId && meId !== value && (
              <CommandGroup>
                <CommandItem value="__me" onSelect={() => pick(meId)}>
                  Assign to me
                </CommandItem>
              </CommandGroup>
            )}
            <CommandGroup heading="Members">
              {members.map(m => (
                <CommandItem key={m.id} value={`${m.name} ${m.email}`} onSelect={() => pick(m.id)}>
                  <MemberAvatar userId={m.id} showName className="flex-1" />
                  {m.id === value && <Check className="ml-auto h-4 w-4" />}
                </CommandItem>
              ))}
            </CommandGroup>
            {value && (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem value="__unassign" onSelect={() => pick(undefined)}>
                    Unassign
                  </CommandItem>
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
