import { headlineOf, priceLabel } from '@/features/plans/currencies/headline'
import { useCurrency } from '@/hooks/useCurrency'

import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PricedOption } from '@/rpc/api/plans/v1/models_pb'

const usePlanHeadline = (priced: PricedOption[]) => headlineOf(priced, useCurrency().currency)

/** A plan's price in the tenant's currency (else the first it is sold in), its other options
 * muted under it. */
export const HeadlinePrice = ({
  priced,
  options,
}: {
  priced: PricedOption[]
  /** The version's options, to name each price in the tooltip. */
  options: OptionLike[]
}) => {
  const headline = usePlanHeadline(priced)
  if (!headline) return null
  const lead = priceLabel(headline.lead)
  const others = headline.others.map(priceLabel).filter(label => label !== undefined)
  const named = [headline.lead, ...headline.others]
    .map(p => {
      const label = priceLabel(p)
      const name = options.find(o => o.id === p.optionId)?.name
      return label && name ? `${name}: ${label}` : label
    })
    .filter(label => label !== undefined)
    .join('\n')
  return (
    <div className="min-w-0" title={options.length > 1 ? named : undefined}>
      <div className="truncate text-sm">
        {lead && <span className="font-semibold tabular-nums">{lead}</span>}
        {lead && headline.lead.amount !== undefined && headline.lead.hasUsage && (
          <span className="text-muted-foreground"> + usage</span>
        )}
      </div>
      {others.length > 0 && (
        <div className="truncate text-xs tabular-nums text-muted-foreground">
          {others.join(' · ')}
        </div>
      )}
    </div>
  )
}

/** "EUR · USD · BRL": the currency shown first. */
export const SoldIn = ({ priced }: { priced: PricedOption[] }) => {
  const headline = usePlanHeadline(priced)
  if (!headline) return null
  return (
    <div className="truncate font-mono text-[11px] text-muted-foreground">
      {headline.currencies.join(' · ')}
    </div>
  )
}
