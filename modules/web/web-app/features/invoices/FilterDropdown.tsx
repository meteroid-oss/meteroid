import { D, F, G } from '@mobily/ts-belt'
import {
  Button,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  Command,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from '@ui2/components'
import { CheckIcon, XIcon, PlusIcon } from 'lucide-react'

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import React from 'react'

interface Props {
  setStatus: (search: InvoiceStatus | undefined) => void
  status?: InvoiceStatus
}

export const FilterDropdown = ({ status, setStatus }: Props) => {
  const [open, setOpen] = React.useState(false)

  const statuses = D.toPairs(InvoiceStatus).filter(([_, status]) => G.isNumber(status))

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" hasIcon className="w-[150px] justify-start">
          {status !== undefined ? (
            <>
              {InvoiceStatus[status].toString()}

              <XIcon
                className="ml-auto h-4 w-4"
                onClick={e => {
                  e.stopPropagation()
                  setStatus(undefined)
                  setOpen(false)
                }}
              />
            </>
          ) : (
            <>
              <PlusIcon size={12} /> Filter
            </>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent useTriggerWidth className="p-0" side="bottom" align="start">
        <Command>
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>
            <CommandGroup>
              {statuses.map(([key, statusOption]) => (
                <CommandItem
                  key={key}
                  value={key}
                  onSelect={() => {
                    setOpen(!open)
                    setStatus(statusOption)
                  }}
                >
                  {key}
                  <CheckIcon
                    className={cn(
                      'ml-auto h-4 w-4',
                      status === statusOption ? 'opacity-100' : 'opacity-0'
                    )}
                  />
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
