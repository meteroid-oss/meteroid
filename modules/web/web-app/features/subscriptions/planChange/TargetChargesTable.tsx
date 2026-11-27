import { Badge, Input } from '@md/ui'
import { cn } from '@ui/lib'
import { ArrowRight, Minus, Plus } from 'lucide-react'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import {
  formatSubscriptionFeeBillingPeriod,
  formatSubscriptionFeeCompact,
  oneAmountPrice,
} from '@/features/subscriptions/utils/fees'

import type {
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import type { PreviewPlanChangeResponse } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { ReactNode } from 'react'

const FeeLabel = ({
  fee,
  period,
  currency,
}: {
  fee?: SubscriptionFee
  period?: SubscriptionFeeBillingPeriod
  currency: string
}) => {
  const creditUnits = useCreditUnits()
  return (
    <span className="text-xs text-muted-foreground">
      {formatSubscriptionFeeCompact(fee, currency, creditUnits)}
      {period !== undefined && (
        <span className="ml-1.5 opacity-70">{formatSubscriptionFeeBillingPeriod(period)}</span>
      )}
    </span>
  )
}

const isPrice = (raw: string) => /^\d+(\.\d+)?$/.test(raw.trim())

/** A price a target charge is set at, over its list price: empty keeps the list price. */
const PriceCell = ({
  componentId,
  fee,
  currency,
  prices,
  onPrice,
}: {
  componentId?: string
  fee?: SubscriptionFee
  currency: string
  prices?: Record<string, string>
  onPrice?: (componentId: string, price: string) => void
}) => {
  if (!onPrice || !componentId || oneAmountPrice(fee) === undefined) return null
  const value = prices?.[componentId] ?? ''
  return (
    <label className="mt-2 flex items-center gap-1.5 text-xs text-muted-foreground">
      <Input
        className="h-8 w-28 text-xs"
        inputMode="decimal"
        placeholder="List price"
        aria-label="Price"
        aria-invalid={value.trim() !== '' && !isPrice(value)}
        value={value}
        onChange={e => onPrice(componentId, e.target.value)}
      />
      {currency}
    </label>
  )
}

/** The prices a plan change sends: those set to a decimal, by target component. */
export const targetPrices = (prices: Record<string, string>) =>
  Object.entries(prices)
    .filter(([, price]) => isPrice(price))
    .map(([componentId, price]) => ({ componentId, price: price.trim() }))

export const pricesKey = (prices: Record<string, string>) =>
  targetPrices(prices)
    .map(p => `${p.componentId}=${p.price}`)
    .sort()
    .join('|')

/**
 * The charges of the plan changed to, as a plan change maps them from those held (R-163): each
 * matched or added one with its seats or level controls and, priced by one amount, its own price;
 * the ones removed. Used by the plan change and by a phase switching plan.
 */
export const TargetChargesTable = ({
  preview,
  currency,
  pending,
  prices,
  onPrice,
  controls,
}: {
  preview: PreviewPlanChangeResponse
  currency: string
  pending?: boolean
  prices?: Record<string, string>
  onPrice?: (componentId: string, price: string) => void
  /** Seat and level choices of a target component. */
  controls?: (componentId: string | undefined, fee?: SubscriptionFee) => ReactNode
}) => (
  <div
    className={cn(
      'bg-card rounded-lg shadow-sm overflow-hidden transition-opacity',
      pending && 'opacity-60'
    )}
  >
    <table className="w-full">
      <thead className="bg-muted/40">
        <tr>
          <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
            Current
          </th>
          <th className="px-2 py-3 w-8" />
          <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
            New
          </th>
          <th className="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase tracking-wider">
            Status
          </th>
        </tr>
      </thead>
      <tbody className="divide-y divide-border">
        {preview.matched.map((match, idx) => (
          <tr key={`matched-${match.priceComponentId ?? idx}`}>
            <td className="px-4 py-3">
              <div className="text-sm text-foreground">{match.currentName}</div>
              <FeeLabel fee={match.currentFee} period={match.currentPeriod} currency={currency} />
            </td>
            <td className="px-2 py-3 text-center">
              <ArrowRight className="h-4 w-4 text-muted-foreground inline-block" />
            </td>
            <td className="px-4 py-3">
              <div className="text-sm font-medium text-foreground">{match.newName}</div>
              <FeeLabel fee={match.newFee} period={match.newPeriod} currency={currency} />
              {controls?.(match.priceComponentId, match.newFee)}
              <PriceCell
                componentId={match.priceComponentId}
                fee={match.newFee}
                currency={currency}
                prices={prices}
                onPrice={onPrice}
              />
            </td>
            <td className="px-4 py-3 text-right">
              <Badge variant="secondary" size="sm">
                Matched
              </Badge>
            </td>
          </tr>
        ))}
        {preview.added.map((added, idx) => (
          <tr key={`added-${added.priceComponentId ?? idx}`}>
            <td className="px-4 py-3 text-sm text-muted-foreground italic">--</td>
            <td className="px-2 py-3 text-center">
              <Plus className="h-4 w-4 text-brand inline-block" />
            </td>
            <td className="px-4 py-3">
              <div className="text-sm font-medium text-foreground">{added.name}</div>
              <FeeLabel fee={added.fee} period={added.period} currency={currency} />
              {controls?.(added.priceComponentId, added.fee)}
              <PriceCell
                componentId={added.priceComponentId}
                fee={added.fee}
                currency={currency}
                prices={prices}
                onPrice={onPrice}
              />
            </td>
            <td className="px-4 py-3 text-right">
              <Badge variant="outline" size="sm" className="text-brand border-brand/30">
                Added
              </Badge>
            </td>
          </tr>
        ))}
        {preview.removed.map((removed, idx) => (
          <tr key={`removed-${idx}`} className="bg-destructive/5">
            <td className="px-4 py-3">
              <div className="text-sm text-foreground line-through">{removed.name}</div>
              <FeeLabel
                fee={removed.currentFee}
                period={removed.currentPeriod}
                currency={currency}
              />
            </td>
            <td className="px-2 py-3 text-center">
              <Minus className="h-4 w-4 text-destructive inline-block" />
            </td>
            <td className="px-4 py-3 text-sm text-muted-foreground italic">--</td>
            <td className="px-4 py-3 text-right">
              <Badge variant="destructive" size="sm">
                Removed
              </Badge>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
    {preview.matched.length === 0 && preview.added.length === 0 && preview.removed.length === 0 && (
      <div className="p-8 text-center text-sm text-muted-foreground">
        No component changes detected between the two plans.
      </div>
    )}
  </div>
)
