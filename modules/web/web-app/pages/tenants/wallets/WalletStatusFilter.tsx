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

import { slotItems } from '@/edition'
import { walletHealthLabel } from '@/features/credit/walletStatus'
import { WalletStatus } from '@/rpc/api/wallets/v1/models_pb'

import type { WalletListFilter } from '@/edition/types'

/** The `?status=` value of a core or module filter entry. */
export type WalletFilter = string

const CORE_FILTERS: WalletListFilter[] = [
  { value: 'healthy', label: 'Has balance', request: { status: WalletStatus.HEALTHY } },
  { value: 'low', label: walletHealthLabel.low, request: { status: WalletStatus.LOW } },
  {
    value: 'depleted',
    label: walletHealthLabel.depleted,
    request: { status: WalletStatus.DEPLETED },
  },
  {
    value: 'blocked',
    label: walletHealthLabel.blocked,
    request: { status: WalletStatus.USAGE_BLOCKED },
  },
]

const walletFilters = (): WalletListFilter[] => [
  ...CORE_FILTERS,
  ...slotItems('walletList.filters'),
]

export const parseWalletFilter = (value: string): WalletFilter | undefined =>
  walletFilters().find(filter => filter.value === value)?.value

export const walletFilterRequest = (
  filter?: WalletFilter
): { status?: WalletStatus; autoTopUpFailing: boolean } => {
  const request = walletFilters().find(entry => entry.value === filter)?.request
  return { status: request?.status, autoTopUpFailing: request?.autoTopUpFailing ?? false }
}

export const WalletStatusFilter = ({
  value,
  onChange,
}: {
  value?: WalletFilter
  onChange: (value: WalletFilter | undefined) => void
}) => {
  const [open, setOpen] = useState(false)
  const filters = walletFilters()
  const selected = filters.find(filter => filter.value === value)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" hasIcon className="text-xs font-medium">
          <span>{selected ? selected.label : 'All statuses'}</span>
          {selected ? (
            <XIcon
              className="h-3 w-3"
              aria-label="Clear status filter"
              onClick={event => {
                event.stopPropagation()
                onChange(undefined)
                setOpen(false)
              }}
            />
          ) : (
            <ChevronDownIcon size={14} />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-56 p-0" side="bottom" align="start">
        <Command>
          <CommandList>
            <CommandGroup>
              {filters.map(filter => (
                <CommandItem
                  key={filter.value}
                  value={filter.label}
                  onSelect={() => {
                    setOpen(false)
                    onChange(filter.value)
                  }}
                >
                  {filter.label}
                  <CheckIcon
                    className={cn(
                      'ml-auto h-4 w-4',
                      value === filter.value ? 'opacity-100' : 'opacity-0'
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
