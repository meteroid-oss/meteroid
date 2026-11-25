import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { cn } from '@ui/lib'

import { byPosition, optionHandle } from '@/features/subscriptions/pricecomponents/planOption'

import type { PlanOption } from '@/rpc/api/plans/v1/models_pb'

/** The plan option bought, by name; nothing to pick for a version with a single option. */
export const OptionSelect = ({
  options,
  value,
  onChange,
  className,
}: {
  options: PlanOption[]
  value: PlanOption | undefined
  onChange: (handle: string) => void
  className?: string
}) => {
  if (options.length < 2) return null
  return (
    <Select value={value ? optionHandle(value) : ''} onValueChange={onChange}>
      <SelectTrigger className={cn('h-9', className)} aria-label="Option">
        <SelectValue placeholder="Option" />
      </SelectTrigger>
      <SelectContent>
        {byPosition(options).map(option => (
          <SelectItem key={option.id} value={optionHandle(option)}>
            {option.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
