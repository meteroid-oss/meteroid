import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  cn,
} from '@md/ui'
import { MoreHorizontalIcon, Trash2Icon } from 'lucide-react'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { optionCell, sameInEveryOption } from '@/features/plans/currencies/priceCells'
import {
  useComponentMetric,
  useComponentProduct,
} from '@/features/plans/pricecomponents/componentRefs'

import { chipParts, countsWords, rowWords } from './billingWords'

import type { ChargeContext } from './billingWords'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

export const useChargeContext = (component: PriceComponent): ChargeContext => {
  const product = useComponentProduct(component.productId)
  const structure = product?.feeStructure?.structure
  const metricId =
    structure?.case === 'usage' || structure?.case === 'capacity'
      ? structure.value.metricId
      : undefined
  const metric = useComponentMetric(metricId)
  return { product, metric, unit: metricUnitFromProto(metric) }
}

type Props = {
  component: PriceComponent
  currency: string
  /** Every currency the plan is priced in: a cell missing in another one is flagged too. */
  currencies: string[]
  options: OptionLike[]
  /** The option picked in the switch: the one shown alone, and what "Edit charge" opens. */
  selected: OptionLike
  /** Every option's price instead of the selected one alone. */
  sideBySide: boolean
  isDraft: boolean
  /** Opens the editor on an option's cell; `merged`: the one cell every option shares. */
  onEdit: (option: OptionLike, merged: boolean) => void
  /** Opens the editor on a missing cell of another currency or option. */
  onEditElsewhere: (currency: string, option: OptionLike) => void
  onRemove: () => void
}

/** "EUR, Annual" where a plan has options; just "EUR" otherwise. */
const cellName = (currency: string, option: OptionLike | undefined, options: OptionLike[]) =>
  option && options.length > 1 ? `${currency}, ${option.name}` : currency

/** A charge of a product: its name, what it is, its price chip(s), and the cells it lacks. */
export const ChargeRow = ({
  component,
  currency,
  currencies,
  options,
  selected,
  sideBySide,
  isDraft,
  onEdit,
  onEditElsewhere,
  onRemove,
}: Props) => {
  const ctx = useChargeContext(component)
  const included = component.prices.length === 0
  const merged = sameInEveryOption(component, options, currency)
  const shown = sideBySide && !merged ? options : [selected]
  const any = options.map(o => optionCell(component, o.id, currency)).find(cell => cell.length > 0)
  const first = any?.[0] ?? component.prices[0]
  const words = any && rowWords(any, currency, ctx)
  const description = included
    ? 'Included without a price'
    : [countsWords(first, ctx), words?.billed].filter(Boolean).join(' · ')

  // The cells it lacks out of sight: other currencies, and the options not shown.
  const elsewhere = included
    ? []
    : currencies.flatMap(c => {
        const missing = options.filter(
          o =>
            optionCell(component, o.id, c).length === 0 &&
            !(c === currency && shown.some(s => s.id === o.id))
        )
        // Lacking in every option: said once, for the currency.
        if (options.length > 1 && missing.length === options.length) {
          return [{ currency: c, option: missing[0], label: c }]
        }
        return missing.map(o => ({ currency: c, option: o, label: cellName(c, o, options) }))
      })

  return (
    <div className="group px-4 py-3">
      <div className="flex min-h-7 items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2">
          <span className="truncate text-sm font-semibold text-foreground">{component.name}</span>
          <span className="text-xs text-muted-foreground">{description}</span>
        </div>
        {isDraft && (
          <div className="flex shrink-0 items-center gap-0.5">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => onEdit(selected, merged)}
            >
              Edit charge
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground"
                  aria-label={`${component.name} actions`}
                >
                  <MoreHorizontalIcon size={14} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={onRemove} className="text-destructive">
                  <Trash2Icon size={12} className="mr-2" /> Remove charge
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}
      </div>

      {included ? (
        <p className="mt-1.5 text-sm text-muted-foreground">
          Included in every currency and option. Its features come with the plan, nothing is
          billed.
        </p>
      ) : (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          {shown.map(option => (
            <span key={option.id} className="inline-flex items-center gap-2">
              {shown.length > 1 && (
                <span className="text-xs text-muted-foreground">{option.name}</span>
              )}
              <PriceChip
                prices={optionCell(component, option.id, currency)}
                currency={currency}
                missing={`No price in ${cellName(currency, option, options)}`}
                ctx={ctx}
                isDraft={isDraft}
                onEdit={() => onEdit(option, merged)}
              />
            </span>
          ))}
          {merged && options.length > 1 && (
            <span className="text-xs text-muted-foreground">Same in every option</span>
          )}
          {elsewhere.slice(0, 2).map(gap => (
            <MissingChip
              key={`${gap.currency}:${gap.option.id}`}
              label={`No price in ${gap.label}`}
              onClick={isDraft ? () => onEditElsewhere(gap.currency, gap.option) : undefined}
            />
          ))}
          {elsewhere.length > 2 && (
            <span className="text-xs text-warning">and {elsewhere.length - 2} more</span>
          )}
        </div>
      )}
    </div>
  )
}

const chip =
  'inline-flex items-baseline gap-1.5 rounded-md border px-2.5 py-1 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

/** The dashed amber chip of a cell without a price; on a draft, a click prices it. */
const MissingChip = ({ label, onClick }: { label: string; onClick?: () => void }) => {
  const style = cn(chip, 'border-dashed border-warning/60 py-0.5 text-xs text-warning')
  return onClick ? (
    <button type="button" onClick={onClick} title="Add this price" className={cn(style, 'hover:bg-warning/10')}>
      {label}
    </button>
  ) : (
    <span className={style}>{label}</span>
  )
}

/** One cell's price as a bordered chip: "$149 / month"; on a draft, a click edits it. */
export const PriceChip = ({
  prices,
  currency,
  missing,
  ctx,
  isDraft,
  onEdit,
}: {
  prices: Price[]
  currency: string
  missing: string
  ctx: ChargeContext
  isDraft: boolean
  onEdit: () => void
}) => {
  const words = rowWords(prices, currency, ctx)
  if (!words) return <MissingChip label={missing} onClick={isDraft ? onEdit : undefined} />
  const { strong, quiet } = chipParts(words.amount)
  const body = (
    <>
      <span className="text-sm font-semibold tabular-nums text-foreground">{strong}</span>
      {quiet && <span className="text-xs text-muted-foreground">{quiet}</span>}
    </>
  )
  return isDraft ? (
    <button
      type="button"
      onClick={onEdit}
      title="Edit this price"
      className={cn(chip, 'border-border bg-background hover:border-muted-foreground/50 hover:bg-accent')}
    >
      {body}
    </button>
  ) : (
    <span className={cn(chip, 'border-border bg-background')}>{body}</span>
  )
}
