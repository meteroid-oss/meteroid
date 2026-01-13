import {
  Button,
  CommandGroup,
  Command,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from '@md/ui'
import { D, G } from '@mobily/ts-belt'
import { CheckIcon, XIcon, PlusIcon } from 'lucide-react'
import { useState } from 'react'

import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'

interface Props {
  setStatus: (search: CreditNoteStatus | undefined) => void
  status?: CreditNoteStatus
}

export const FilterDropdown = ({ status, setStatus }: Props) => {
  const [open, setOpen] = useState(false)

  const statuses = D.toPairs(CreditNoteStatus).filter(([_, status]) => G.isNumber(status))

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="ghost" hasIcon className="w-[150px] justify-start">
          {status !== undefined ? (
            <>
              {CreditNoteStatus[status].toString()}

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
