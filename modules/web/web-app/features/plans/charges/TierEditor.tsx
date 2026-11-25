import { Button, Input, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { HelpCircleIcon, PlusIcon, XIcon } from 'lucide-react'
import { useController } from 'react-hook-form'

import { setTierEnd, splitLastTier } from './kinds'

import type { TierMode, TierRowLike } from './kinds'
import type { ReactNode } from 'react'
import type { Control, FieldValues } from 'react-hook-form'

const grouped = (value: unknown) => {
  const n = Number(String(value ?? ''))
  return Number.isFinite(n) ? n.toLocaleString('en-US') : String(value ?? '')
}
const digits = (value: string) => value.replace(/[^\d]/g, '')

/**
 * Tiers edited by their last unit. Graduated or volume is the form's "Priced" choice, and what a
 * customer pays shows in its invoice preview.
 */
export const TierEditor = ({
  control,
  currency,
  mode,
  unitSingular = 'unit',
}: {
  control: Control<FieldValues>
  currency: string
  mode: TierMode
  unitSingular?: string
}) => {
  const { field, fieldState } = useController({ control, name: 'rows' })
  const rows = (field.value as TierRowLike[] | undefined) ?? []
  const update = (next: TierRowLike[]) => field.onChange(next)
  const patch = (index: number, change: Partial<TierRowLike>) =>
    update(rows.map((row, i) => (i === index ? { ...row, ...change } : row)))

  const rowsError =
    fieldState.error?.message ??
    (Array.isArray(fieldState.error)
      ? (fieldState.error as { firstUnit?: { message?: string } }[]).find(e => e?.firstUnit)
          ?.firstUnit?.message
      : undefined)

  return (
    <div className="space-y-4">
      <div className="overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                <Help
                  label="Units"
                  text={
                    mode === 'graduated'
                      ? 'You set where a tier ends; the next one starts right after it.'
                      : 'You set where the next tier starts: a total that reaches it is priced there.'
                  }
                />
              </th>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                Price per {unitSingular}
              </th>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                <Help label="Flat fee" text="Added once when usage enters this tier" />
              </th>
              <th scope="col" className="px-3 py-2 text-left font-medium">
                <Help label="Cap" text="This tier never costs more than this, whatever the usage" />
              </th>
              <th scope="col" className="w-10 px-2 py-2">
                <span className="sr-only">Remove</span>
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {rows.map((row, index) => {
              const last = index === rows.length - 1
              // Graduated: "1 to [100]", its own units. Volume: "Under [100]", the totals it
              // prices, the next tier pricing the one typed (billing-core volume_charge).
              const graduated = mode === 'graduated'
              const start = Number(row.firstUnit) + (graduated ? 1 : 0)
              return (
                <tr key={index}>
                  <td className="whitespace-nowrap px-3 py-1.5">
                    <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                      {last ? (
                        <>
                          <span className="font-mono tabular-nums text-foreground">
                            {grouped(start)}
                          </span>
                          and above
                        </>
                      ) : (
                        <>
                          {graduated || start > 0 ? (
                            <>
                              <span className="font-mono tabular-nums text-foreground">
                                {grouped(start)}
                              </span>
                              {graduated ? 'to' : 'to under'}
                            </>
                          ) : (
                            'Under'
                          )}
                          <Input
                            inputMode="numeric"
                            aria-label={
                              graduated
                                ? `Last ${unitSingular} of tier ${index + 1}`
                                : `Where tier ${index + 2} starts`
                            }
                            className="h-8 w-28 font-mono tabular-nums"
                            value={grouped(rows[index + 1].firstUnit)}
                            onChange={e => update(setTierEnd(rows, index, digits(e.target.value)))}
                          />
                        </>
                      )}
                    </span>
                  </td>
                  <td className="px-3 py-1.5">
                    <MoneyCell
                      currency={currency}
                      label={`Price per ${unitSingular}, tier ${index + 1}`}
                      value={row.unitPrice}
                      onChange={unitPrice => patch(index, { unitPrice })}
                    />
                  </td>
                  <td className="px-3 py-1.5">
                    <MoneyCell
                      currency={currency}
                      label={`Flat fee, tier ${index + 1}`}
                      placeholder="none"
                      value={row.flatFee}
                      onChange={flatFee => patch(index, { flatFee: flatFee || undefined })}
                    />
                  </td>
                  <td className="px-3 py-1.5">
                    <MoneyCell
                      currency={currency}
                      label={`Cap, tier ${index + 1}`}
                      placeholder="none"
                      value={row.flatCap}
                      onChange={flatCap => patch(index, { flatCap: flatCap || undefined })}
                    />
                  </td>
                  <td className="px-2 py-1.5 text-right">
                    {index > 0 && rows.length > 2 && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="size-7"
                        aria-label={`Remove tier ${index + 1}`}
                        onClick={() => update(rows.filter((_, i) => i !== index))}
                      >
                        <XIcon size={12} />
                      </Button>
                    )}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <Button type="button" variant="outline" size="sm" hasIcon onClick={() => update(splitLastTier(rows))}>
          <PlusIcon size={12} /> Split the last tier
        </Button>
      </div>
      {rowsError && <p className="text-xs text-destructive">{rowsError}</p>}
    </div>
  )
}

/** A label with its (?): what it means, on hover. */
export const Help = ({
  label,
  text,
  wide = false,
}: {
  label: string
  text: ReactNode
  /** A longer explanation, in a wider tooltip. */
  wide?: boolean
}) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span className="inline-flex cursor-help items-center gap-1">
        {label}{' '}
        <HelpCircleIcon size={11} aria-label={typeof text === 'string' ? text : `About ${label}`} />
      </span>
    </TooltipTrigger>
    <TooltipContent className={wide ? 'max-w-80 text-xs' : 'max-w-56'}>{text}</TooltipContent>
  </Tooltip>
)

/** An amount in a table cell, its currency code before it. */
export const MoneyCell = ({
  currency,
  label,
  value,
  placeholder,
  onChange,
}: {
  currency: string
  label: string
  value?: string
  placeholder?: string
  onChange: (value: string) => void
}) => (
  <div className="flex h-8 w-28 items-center gap-1 rounded-md border border-input bg-background px-2 focus-within:ring-1 focus-within:ring-ring">
    <span className="font-mono text-[11px] text-muted-foreground">{currency}</span>
    <input
      inputMode="decimal"
      aria-label={label}
      placeholder={placeholder}
      className="min-w-0 flex-1 bg-transparent text-right font-mono text-sm tabular-nums outline-none placeholder:text-muted-foreground/60"
      value={value ?? ''}
      onChange={e => onChange(e.target.value.replace(/[^\d.]/g, ''))}
    />
  </div>
)
