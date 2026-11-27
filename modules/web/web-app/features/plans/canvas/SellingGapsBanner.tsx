import { skipToken } from '@connectrpc/connect-query'
import { Tooltip, TooltipContent, TooltipTrigger, cn } from '@md/ui'
import { useSetAtom } from 'jotai'
import { AlertTriangleIcon, ArrowRightIcon, HelpCircleIcon } from 'lucide-react'
import { useState } from 'react'

import { orderCurrencies, pricedCurrencies } from '@/features/plans/currencies/priceCells'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { priceCellRequestAtom } from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import { gapParts, gapSentence, sellingGaps } from './planLines'

import type { SellingGap } from './planLines'

const SHOWN = 4

/**
 * What keeps the plan from selling, one chip per currency and option lacking a price: "BRL,
 * Annual · Platform fee +2". On a draft, a chip opens the first missing price in the editor.
 */
export const SellingGapsBanner = () => {
  const { plan, version } = usePlanWithVersion()
  const isDraft = useIsDraftVersion()
  const request = useSetAtom(priceCellRequestAtom)
  const [all, setAll] = useState(false)
  const components = useQuery(
    listPriceComponents,
    version ? { planVersionId: version.id } : skipToken
  ).data?.components
  if (!plan || !version || !components) return null

  const options = version.options
  const currencies = orderCurrencies(
    [version.currency, ...pricedCurrencies(components)],
    version.currency
  )
  const gaps = sellingGaps(components, options, currencies)
  if (!gaps.length) return null
  // A gap without an option stands for every option of its currency.
  const blocked = gaps.reduce((n, gap) => n + (gap.option ? 1 : options.length), 0)
  const ready = isDraft && blocked < currencies.length * options.length
  // Prices are edited in a draft's pricing section, the only place a chip can lead to.
  const editable = isDraft && version.usesProductPricing
  const shown = all ? gaps : gaps.slice(0, SHOWN)

  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border border-warning/40 bg-warning/10 px-4 py-2 text-sm text-foreground"
    >
      <span className="inline-flex items-center gap-2 font-medium">
        <AlertTriangleIcon size={15} className="shrink-0 text-warning" />
        Missing prices
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="text-muted-foreground hover:text-foreground"
              aria-label="About missing prices"
            >
              <HelpCircleIcon size={13} />
            </button>
          </TooltipTrigger>
          <TooltipContent multiline className="max-w-64 text-xs font-normal">
            {plan.name} is sold in a currency and option once every charge with a price has one
            there.
          </TooltipContent>
        </Tooltip>
      </span>
      <ul aria-label="Missing prices" className="flex flex-wrap items-center gap-1.5">
        {shown.map(gap => (
          <li key={`${gap.currency}:${gap.option?.id ?? ''}`}>
            <GapChip
              gap={gap}
              planName={plan.name}
              onPrice={
                editable
                  ? () =>
                      request({
                        componentId: gap.charges[0].id,
                        currency: gap.currency,
                        optionId: gap.option?.id,
                      })
                  : undefined
              }
            />
          </li>
        ))}
        {gaps.length > shown.length && (
          <li>
            <button
              type="button"
              onClick={() => setAll(true)}
              className="text-xs text-muted-foreground hover:text-foreground hover:underline"
            >
              {gaps.length - shown.length} more
            </button>
          </li>
        )}
      </ul>
      {ready && (
        <span className="ml-auto text-xs text-muted-foreground">
          Everything else is ready to publish.
        </span>
      )}
    </div>
  )
}

/** "BRL, Annual · Platform fee +2", the whole sentence on hover; a click prices the first. */
const GapChip = ({
  gap,
  planName,
  onPrice,
}: {
  gap: SellingGap
  planName: string
  onPrice?: () => void
}) => {
  const [first, ...others] = gap.charges
  const style =
    'inline-flex items-center gap-1.5 rounded-md border border-dashed border-warning/60 bg-background/60 px-2 py-0.5 text-xs'
  const body = (
    <>
      <span className="font-medium text-foreground">{gapParts(gap, planName).where}</span>
      <span className="text-muted-foreground">
        {first.name}
        {others.length > 0 && ` +${others.length}`}
      </span>
      {onPrice && <ArrowRightIcon size={11} className="text-warning" />}
    </>
  )
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {onPrice ? (
          <button
            type="button"
            onClick={onPrice}
            className={cn(
              style,
              'transition-colors hover:bg-warning/15 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'
            )}
          >
            {body}
          </button>
        ) : (
          <span tabIndex={0} className={style}>
            {body}
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent multiline className="max-w-72 text-xs">
        {gapSentence(gap, planName)}
        {onPrice && ` Click to price ${first.name}.`}
      </TooltipContent>
    </Tooltip>
  )
}
