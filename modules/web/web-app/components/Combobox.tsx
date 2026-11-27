import {
  Button,
  cn,
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
} from '@md/ui'
import { CaretSortIcon, CheckIcon } from '@radix-ui/react-icons'
import { useState } from 'react'

interface Props {
  options: { label: React.ReactNode; value: string; keywords?: string[] }[]
  description?: string
  className?: string
  hasSearch?: boolean
  placeholder?: string
  action?: React.ReactNode
  unit?: string
  value: string | undefined
  onChange: (value: string) => void
}
export function Combobox({
  options,
  className,
  hasSearch,
  placeholder,
  action,
  value,
  onChange,
  unit = '...',
}: Props) {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className={cn(
            // Drawn as a text field (see Input).
            'flex h-8 w-full items-center justify-between whitespace-nowrap rounded-md border border-field-border bg-field px-3 py-1 text-[13px] font-normal text-field-foreground shadow-none transition-[border-color,box-shadow] hover:bg-field placeholder:text-field-placeholder focus:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8 disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
            className
            //!field.value && ''
          )}
        >
          {value
            ? options.find(option => option.value === value)?.label
            : placeholder ?? `Select ${unit}`}
          <CaretSortIcon className="ml-2 h-4 w-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[var(--radix-popover-trigger-width)] p-0 max-h-9 ">
        <Command className="border border-border ">
          {hasSearch && (
            <>
              <CommandInput placeholder={`Search ${unit}`} className="h-9  " />
              <CommandEmpty>No data found.</CommandEmpty>
            </>
          )}

          <CommandList>
            {options.map((option, index) => (
              <CommandItem
                value={`${option.value}/${option.keywords?.join(' ')}`}
                key={option.value}
                keywords={option.keywords}
                autoFocus={index === 0}
                onSelect={() => {
                  onChange(option.value)
                  setOpen(false)
                }}
              >
                {option.label}
                <CheckIcon
                  className={cn(
                    'ml-auto h-4 w-4',
                    option.value === value ? 'opacity-100' : 'opacity-0'
                  )}
                />
              </CommandItem>
            ))}
            {!options.length && (
              <CommandGroup>
                <CommandItem disabled>No data.</CommandItem>
              </CommandGroup>
            )}
            {action && (
              <>
                <CommandSeparator />
                <div
                  className="h-8 relative flex cursor-default select-none items-center rounded-sm pt-1 text-sm outline-none"
                  onClick={() => setOpen(false)}
                >
                  {action}
                </div>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
