import { Input, Label, Switch } from '@md/ui'
import { useId, useState } from 'react'

import { Segmented } from '@/features/plans/charges/ChargeComposer'

// How an add-on is sold, beside its price: how many a subscription takes, and whether customers
// add it themselves. Saved with the new add-on, or at once from its panel.

type Mode = 'one' | 'several' | 'any'

const modeOf = (max: number | undefined): Mode =>
  max === undefined ? 'any' : max === 1 ? 'one' : 'several'

/** "One", "Up to 5", "Any number" per subscription; none: any number. */
export const PerSubscriptionField = ({
  value,
  onChange,
  disabled,
}: {
  value: number | undefined
  onChange: (max: number | undefined) => void
  disabled?: boolean
}) => {
  const mode = modeOf(value)
  // Typed freely, committed when left: the panel saves each change.
  const [draft, setDraft] = useState<string>()
  const commit = () => {
    if (draft === undefined) return
    const max = Math.max(2, parseInt(draft, 10) || 2)
    setDraft(undefined)
    if (max !== value) onChange(max)
  }
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <span className="w-36 shrink-0 text-xs text-muted-foreground">Per subscription</span>
      <Segmented<Mode>
        label="Per subscription"
        value={mode}
        options={[
          { value: 'one', label: 'One', disabled },
          { value: 'several', label: 'Up to', disabled },
          { value: 'any', label: 'Any number', disabled },
        ]}
        onChange={next =>
          onChange(next === 'one' ? 1 : next === 'several' ? Math.max(value ?? 0, 2) : undefined)
        }
      />
      {mode === 'several' && (
        <Input
          type="number"
          min={2}
          aria-label="At most, per subscription"
          disabled={disabled}
          value={draft ?? String(value ?? 2)}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => e.key === 'Enter' && commit()}
          className="h-7 w-16 text-sm"
        />
      )}
    </div>
  )
}

export const SelfServiceField = ({
  value,
  onChange,
  disabled,
}: {
  value: boolean
  onChange: (selfServiceable: boolean) => void
  disabled?: boolean
}) => {
  const id = useId()
  return (
    <div className="flex items-center gap-3">
      <Label htmlFor={id} className="w-36 shrink-0 text-xs font-normal text-muted-foreground">
        Customers add it
      </Label>
      <Switch id={id} checked={value} disabled={disabled} onCheckedChange={onChange} />
      <span className="text-xs text-muted-foreground">
        {value ? 'Themselves, from their portal' : 'No, your team adds it'}
      </span>
    </div>
  )
}
