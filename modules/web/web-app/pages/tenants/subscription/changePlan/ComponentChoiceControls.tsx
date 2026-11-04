import { Input, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { useEffect, useRef, useState } from 'react'

import { useDebouncedCallback } from '@/hooks/useDebounce'
import { formatCadence } from '@/lib/mapping/prices'
import { formatCurrencyNoRounding, formatUsage } from '@/lib/utils/numbers'

import type {
  ComponentChoice,
  ConfigurableComponent,
} from '@/pages/tenants/subscription/changePlan/componentChoices'

const capacityLabel = (
  { component, choice }: ConfigurableComponent,
  included: bigint,
  currency: string
) => {
  const price = component.prices.find(
    p =>
      p.pricing.case === 'capacityPricing' &&
      p.pricing.value.included === included &&
      (choice.billingPeriod === undefined || p.cadence === choice.billingPeriod)
  )
  const amount =
    price?.pricing.case === 'capacityPricing' && !price.creditUnit
      ? ` · ${formatCurrencyNoRounding(price.pricing.value.rate, currency)}`
      : ''
  return `${formatUsage(Number(included))} included${amount}`
}

const SeatsInput = ({
  value,
  min,
  max,
  unit,
  onCommit,
}: {
  value: number
  min: number
  max?: number
  unit: string
  onCommit: (seats: number) => void
}) => {
  const [draft, setDraft] = useState(String(value))
  const ref = useRef<HTMLInputElement>(null)
  const commit = useDebouncedCallback((raw: string) => {
    const seats = parseInt(raw, 10)
    if (!Number.isNaN(seats)) onCommit(seats)
  }, 400)

  useEffect(() => {
    if (document.activeElement !== ref.current) setDraft(String(value))
  }, [value])

  const settle = () => {
    const seats = parseInt(draft, 10)
    if (Number.isNaN(seats)) return setDraft(String(value))
    const clamped = Math.min(Math.max(seats, min), max ?? seats)
    setDraft(String(clamped))
    onCommit(clamped)
  }

  return (
    <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Input
        ref={ref}
        type="number"
        min={min}
        max={max}
        className="h-8 w-20 text-xs"
        value={draft}
        onChange={e => {
          setDraft(e.target.value)
          commit(e.target.value)
        }}
        onBlur={settle}
      />
      {unit}
    </label>
  )
}

export const ComponentChoiceControls = ({
  configurable,
  currency,
  slotUnit,
  onChange,
}: {
  configurable: ConfigurableComponent
  currency: string
  slotUnit?: string
  onChange: (choice: ComponentChoice) => void
}) => {
  const { cadences, capacities, slot, choice } = configurable

  return (
    <div className="mt-2 flex flex-wrap items-center gap-2">
      {cadences.length > 0 && (
        <Select
          value={String(choice.billingPeriod)}
          onValueChange={v => onChange({ billingPeriod: Number(v) })}
        >
          <SelectTrigger className="h-8 w-auto text-xs" aria-label="Billing cadence">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {cadences.map(c => (
              <SelectItem key={c} value={String(c)}>
                {formatCadence(c)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      {capacities.length > 0 && (
        <Select
          value={String(choice.committedCapacity)}
          onValueChange={v => onChange({ committedCapacity: BigInt(v) })}
        >
          <SelectTrigger className="h-8 w-auto text-xs" aria-label="Committed capacity">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {capacities.map(c => (
              <SelectItem key={String(c)} value={String(c)}>
                {capacityLabel(configurable, c, currency)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      {slot && choice.initialSlotCount !== undefined && (
        <SeatsInput
          value={choice.initialSlotCount}
          min={slot.minSlots ?? 0}
          max={slot.maxSlots}
          unit={slotUnit || 'seats'}
          onCommit={initialSlotCount => onChange({ initialSlotCount })}
        />
      )}
    </div>
  )
}
