import { Badge } from '@md/ui'
import { createElement } from 'react'

import { feeTypeIcon } from '@/features/plans/pricecomponents/utils'

import type { ComponentFeeType } from '@/features/pricing/conversions'

interface FeeTypeOption {
  type: ComponentFeeType
  label: string
  description: string
  disabled?: boolean
}

const feeTypeOptions: FeeTypeOption[] = [
  { type: 'rate', label: 'Subscription Rate', description: 'Fixed rate per billing period' },
  { type: 'slot', label: 'Slot-based', description: 'Seats, licenses or purchasable units' },
  { type: 'capacity', label: 'Capacity', description: 'Committed capacity with overage' },
  { type: 'usage', label: 'Usage-based', description: 'Charge based on metered usage' },
  { type: 'oneTime', label: 'One-time', description: 'Single charge at subscription start' },
  { type: 'extraRecurring', label: 'Recurring charge', description: 'Additional recurring fee' },
]

interface FeeTypePickerProps {
  onSelect: (feeType: ComponentFeeType) => void
}

export const FeeTypePicker = ({ onSelect }: FeeTypePickerProps) => {
  return (
    <div className="grid grid-cols-2 gap-3">
      {feeTypeOptions.map(option => (
        <button
          key={option.type}
          type="button"
          disabled={option.disabled}
          onClick={() => onSelect(option.type)}
          className="flex flex-col items-start gap-2 rounded-lg border border-border bg-card p-4 text-left transition-colors hover:bg-accent disabled:cursor-default disabled:opacity-50"
        >
          <div className="flex w-full items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="text-muted-foreground">{createElement(feeTypeIcon(option.type), { size: 20 })}</span>
              <span className="text-sm font-medium">{option.label}</span>
            </div>
            {option.disabled && (
              <Badge variant="secondary" className="text-xs">
                soon
              </Badge>
            )}
          </div>
          <span className="text-xs text-muted-foreground">{option.description}</span>
        </button>
      ))}
    </div>
  )
}
