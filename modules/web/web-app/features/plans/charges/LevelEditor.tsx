import { Button, Input } from '@md/ui'
import { PlusIcon, XIcon } from 'lucide-react'
import { useController } from 'react-hook-form'

import { Help, MoneyCell } from './TierEditor'
import { addLevel, percentOf } from './kinds'

import type { LevelRow } from './kinds'
import type { Control, FieldValues } from 'react-hook-form'

const grouped = (value: unknown) => {
  const raw = String(value ?? '')
  if (raw === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) ? n.toLocaleString('en-US') : raw
}
const digits = (value: string) => value.replace(/[^\d]/g, '')

/** The first message of the levels' errors: the list's own, else a level's field's. */
const levelsError = (error: unknown): string | undefined => {
  if (!error || typeof error !== 'object') return undefined
  const { message, root } = error as { message?: string; root?: { message?: string } }
  if (message || root?.message) return message || root?.message
  if (!Array.isArray(error)) return undefined
  for (const level of error as (Record<string, { message?: string }> | undefined)[]) {
    const message = level && Object.values(level).find(field => field?.message)?.message
    if (message) return message
  }
  return undefined
}

/**
 * A capacity charge's levels (`thresholds`): each the usage it includes, its fee per period and
 * the rate of the usage above it, the customer picking one.
 */
export const LevelEditor = ({
  control,
  currency,
  per,
  unitPlural,
  perUnit,
  many,
  moneyUnit,
}: {
  control: Control<FieldValues>
  currency: string
  /** The period a fee is for: "month". */
  per: string
  unitPlural: string
  /** What overage is priced per: "per call". */
  perUnit: string
  /** Levels may be added and removed (a plan's); otherwise the one level is priced. */
  many: boolean
  /** The metric counts money in `currency`: the overage rate also reads as a percentage. */
  moneyUnit?: boolean
}) => {
  const { field, fieldState } = useController({ control, name: 'thresholds' })
  const levels = (field.value as LevelRow[] | undefined) ?? []
  const update = (next: LevelRow[]) => field.onChange(next)
  const patch = (index: number, change: Partial<LevelRow>) =>
    update(levels.map((level, i) => (i === index ? { ...level, ...change } : level)))
  const error = levelsError(fieldState.error)
  const several = levels.length > 1

  return (
    <div className="space-y-3">
      <div className="overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                {several ? 'Level' : 'Includes'}
              </th>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                Price / {per}
              </th>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                <Help
                  label={`Overage ${perUnit}`}
                  text={`Usage above the level, billed after each ${per}.`}
                />
              </th>
              {many && (
                <th scope="col" className="w-10 px-2 py-2">
                  <span className="sr-only">Remove</span>
                </th>
              )}
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {levels.map((level, index) => (
              <tr key={index}>
                <td className="whitespace-nowrap px-3 py-1.5">
                  <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                    Up to
                    <Input
                      inputMode="numeric"
                      aria-label={`Usage included, level ${index + 1}`}
                      className="h-8 w-32 font-mono tabular-nums"
                      value={grouped(level.included)}
                      onChange={e => {
                        const raw = digits(e.target.value)
                        patch(index, { included: raw === '' ? undefined : Number(raw) })
                      }}
                    />
                    <span className="text-xs">{unitPlural}</span>
                  </span>
                </td>
                <td className="px-3 py-1.5">
                  <MoneyCell
                    currency={currency}
                    label={`Price per ${per}, level ${index + 1}`}
                    value={level.rate}
                    onChange={rate => patch(index, { rate })}
                  />
                </td>
                <td className="whitespace-nowrap px-3 py-1.5">
                  <span className="inline-flex items-center gap-2">
                    <MoneyCell
                      currency={currency}
                      label={`Overage ${perUnit}, level ${index + 1}`}
                      value={level.overageRate}
                      onChange={overageRate => patch(index, { overageRate })}
                    />
                    {moneyUnit && level.overageRate && (
                      <span className="text-xs text-muted-foreground">
                        {percentOf(level.overageRate)}
                      </span>
                    )}
                  </span>
                </td>
                {many && (
                  <td className="px-2 py-1.5 text-right">
                    {several && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="size-7"
                        aria-label={`Remove level ${index + 1}`}
                        onClick={() => update(levels.filter((_, i) => i !== index))}
                      >
                        <XIcon size={12} />
                      </Button>
                    )}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {many && (
        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            hasIcon
            onClick={() => update(addLevel(levels))}
          >
            <PlusIcon size={12} /> Add a level
          </Button>
          <span className="text-xs text-muted-foreground">
            The customer picks a level when they subscribe.
          </span>
        </div>
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}
