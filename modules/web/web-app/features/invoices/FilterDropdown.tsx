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

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

const STATUS_LABELS: Record<InvoiceStatus, string> = {
  [InvoiceStatus.DRAFT]: 'Draft',
  [InvoiceStatus.FINALIZED]: 'Finalized',
  [InvoiceStatus.UNCOLLECTIBLE]: 'Uncollectible',
  [InvoiceStatus.VOID]: 'Void',
}

const STATUSES: { label: string; value: InvoiceStatus }[] = [
  { label: 'Draft', value: InvoiceStatus.DRAFT },
  { label: 'Finalized', value: InvoiceStatus.FINALIZED },
  { label: 'Uncollectible', value: InvoiceStatus.UNCOLLECTIBLE },
  { label: 'Void', value: InvoiceStatus.VOID },
]

interface Props {
  setStatus: (search: InvoiceStatus | undefined) => void
  status?: InvoiceStatus
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
