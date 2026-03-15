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
import { CheckIcon, ChevronDownIcon, XIcon } from 'lucide-react'
import { useState } from 'react'

import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'

const STATUS_LABELS: Record<CreditNoteStatus, string> = {
  [CreditNoteStatus.DRAFT]: 'Draft',
  [CreditNoteStatus.FINALIZED]: 'Finalized',
  [CreditNoteStatus.VOIDED]: 'Voided',
}

const STATUSES: { label: string; value: CreditNoteStatus }[] = [
  { label: 'Draft', value: CreditNoteStatus.DRAFT },
  { label: 'Finalized', value: CreditNoteStatus.FINALIZED },
  { label: 'Voided', value: CreditNoteStatus.VOIDED },
]

interface Props {
  setStatus: (search: CreditNoteStatus | undefined) => void
  status?: CreditNoteStatus
}

export const FilterDropdown = ({ status, setStatus }: Props) => {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" hasIcon className="text-xs font-medium">
          <span>{status !== undefined ? STATUS_LABELS[status] : 'All statuses'}</span>
          {status !== undefined ? (
            <XIcon
              className="h-3 w-3"
              onClick={e => {
                e.stopPropagation()
                setStatus(undefined)
                setOpen(false)
              }}
            />
          ) : (
            <ChevronDownIcon size={14} />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent useTriggerWidth className="p-0" side="bottom" align="start">
        <Command>
          <CommandList>
            <CommandGroup>
              {STATUSES.map(({ label, value }) => (
                <CommandItem
                  key={value}
                  value={label}
                  onSelect={() => {
                    setOpen(false)
                    setStatus(value)
                  }}
                >
                  {label}
                  <CheckIcon
                    className={cn(
                      'ml-auto h-4 w-4',
                      status === value ? 'opacity-100' : 'opacity-0'
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
