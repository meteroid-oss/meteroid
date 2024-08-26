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
            'flex h-9 w-full border border-border items-center justify-between whitespace-nowrap rounded-md font-normal  bg-transparent hover:bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
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
