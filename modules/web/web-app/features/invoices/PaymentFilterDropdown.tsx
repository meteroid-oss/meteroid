import {
  Button,
  Command,
  CommandGroup,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from '@md/ui'
import { CheckIcon, ChevronDownIcon, XIcon } from 'lucide-react'
import { useState } from 'react'

import { InvoicePaymentStatus } from '@/rpc/api/invoices/v1/models_pb'

const STATUS_LABELS: Record<InvoicePaymentStatus, string> = {
  [InvoicePaymentStatus.UNPAID]: 'Unpaid',
  [InvoicePaymentStatus.PROCESSING]: 'Processing',
  [InvoicePaymentStatus.PARTIALLY_PAID]: 'Partially paid',
  [InvoicePaymentStatus.PAID]: 'Paid',
  [InvoicePaymentStatus.ERRORED]: 'Errored',
}

const STATUSES: { label: string; value: InvoicePaymentStatus }[] = [
  { label: 'Unpaid', value: InvoicePaymentStatus.UNPAID },
  { label: 'Processing', value: InvoicePaymentStatus.PROCESSING },
  { label: 'Partially paid', value: InvoicePaymentStatus.PARTIALLY_PAID },
  { label: 'Paid', value: InvoicePaymentStatus.PAID },
  { label: 'Errored', value: InvoicePaymentStatus.ERRORED },
]

interface Props {
  setStatus: (status: InvoicePaymentStatus | undefined) => void
  status?: InvoicePaymentStatus
}

export const PaymentFilterDropdown = ({ status, setStatus }: Props) => {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" hasIcon className="text-xs font-medium">
          <span>{status !== undefined ? STATUS_LABELS[status] : 'All payments'}</span>
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
