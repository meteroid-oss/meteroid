import { Badge, cn } from '@md/ui'
import { MessageSquarePlus } from 'lucide-react'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import {
  formatSubscriptionFee,
  formatSubscriptionFeeBillingPeriod,
} from '@/features/subscriptions/utils/fees'
import { QuoteAddOn, QuoteDisplayOptions } from '@/rpc/api/quotes/v1/models_pb'
import { SubscriptionFee, SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { formatCurrencyNoRounding } from '@/utils/numbers'

export interface QuoteLine {
  id: string
  name: string
  period: SubscriptionFeeBillingPeriod
  fee?: SubscriptionFee
  listFee?: SubscriptionFee
  description?: string
  sortOrder?: number
  discountPercent?: string
  exampleUsageQuantity?: string
  exampleUsageAmount?: string
  /** What comments name the line by, when it differs from the displayed name. */
  anchor?: string
}

const FEE_TYPE_LABELS: Record<string, string> = {
  rate: 'Rate',
  oneTime: 'One Time',
  recurring: 'Recurring',
  capacity: 'Capacity',
  slot: 'Slot',
  usage: 'Usage',
  credit: 'Credit',
}

const isZero = (value?: string) => !value || Number(value) === 0

export const isZeroFee = (fee?: SubscriptionFee): boolean => {
  const f = fee?.fee
  switch (f?.case) {
    case 'rate':
      return isZero(f.value.rate)
    case 'oneTime':
    case 'recurring':
      return isZero(f.value.total || f.value.rate)
    case 'credit':
      return isZero(f.value.total || f.value.rate) && isZero(f.value.creditAmount)
    case 'capacity':
      return isZero(f.value.rate) && isZero(f.value.overageRate)
    case 'slot':
      return isZero(f.value.unitRate)
    default:
      return false
  }
}

export const formatDiscount = (percent: string) => `−${Number(percent)}%`

export const addOnLine = (addOn: QuoteAddOn): QuoteLine => ({
  id: addOn.id,
  name: addOn.name + (addOn.quantity > 1 ? ` (x${addOn.quantity})` : ''),
  period: addOn.period,
  fee: addOn.fee,
  listFee: addOn.listFee,
  description: addOn.description,
  sortOrder: addOn.sortOrder,
  discountPercent: addOn.discountPercent,
  anchor: addOn.name,
})

export const displayedLines = (lines: QuoteLine[], options?: QuoteDisplayOptions) =>
  lines
    .map((line, index) => ({ line, index }))
    .sort((a, b) => (a.line.sortOrder ?? 0) - (b.line.sortOrder ?? 0) || a.index - b.index)
    .map(({ line }) => line)
    .filter(line => !options?.hideZeroLines || !isZeroFee(line.fee))

const LinePrice = ({
  line,
  currency,
  showListPrice,
}: {
  line: QuoteLine
  currency: string
  showListPrice: boolean
}) => {
  const creditUnits = useCreditUnits()
  if (!line.fee?.fee.case) return <span className="text-muted-foreground">No fee information</span>

  const formatted = formatSubscriptionFee(line.fee, currency, creditUnits)
  const discounted = line.discountPercent && Number(line.discountPercent) > 0
  const listPrice =
    discounted && showListPrice && line.listFee
      ? formatSubscriptionFee(line.listFee, currency, creditUnits).amount
      : undefined
  const quantity = Number(line.exampleUsageQuantity)

  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        {listPrice && (
          <span className="text-xs line-through text-muted-foreground">{listPrice}</span>
        )}
        <span className="font-medium text-foreground text-sm">{formatted.amount}</span>
        {discounted && (
          <Badge variant="secondary" size="sm">
            {formatDiscount(line.discountPercent!)}
          </Badge>
        )}
      </div>
      {line.fee.fee.case === 'credit' && (
        <div className="text-xs text-muted-foreground">{formatted.details}</div>
      )}
      {formatted.breakdown && (
        <div className="text-xs text-muted-foreground whitespace-pre-line mt-1 pl-2 border-l-2 border-muted">
          {formatted.breakdown}
        </div>
      )}
      {line.exampleUsageQuantity && Number.isFinite(quantity) && line.exampleUsageAmount && (
        <div className="text-xs text-muted-foreground mt-1 italic">
          e.g. {quantity.toLocaleString()} units ={' '}
          {formatCurrencyNoRounding(line.exampleUsageAmount, currency)}
        </div>
      )}
    </div>
  )
}

export const lineAnchor = (line: QuoteLine) => (line.anchor ?? line.name).trim()

export const CommentPin = ({ count, className }: { count: number; className?: string }) => (
  <span
    aria-hidden
    className={cn(
      'inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full bg-brand px-1 text-[11px] font-semibold leading-none text-brand-foreground',
      className
    )}
  >
    {count}
  </span>
)

const LineName = ({
  line,
  commentCount,
  onSelect,
}: {
  line: QuoteLine
  commentCount: number
  onSelect?: (lineName: string) => void
}) => {
  const pin = commentCount > 0 && <CommentPin count={commentCount} />
  if (!onSelect) {
    return (
      <div className="flex items-center gap-2 font-medium text-foreground">
        {line.name}
        {pin}
      </div>
    )
  }
  const label =
    commentCount > 0
      ? `${line.name}: ${commentCount} ${commentCount === 1 ? 'message' : 'messages'}, discuss this line`
      : `Discuss ${line.name}`
  return (
    <button
      type="button"
      onClick={() => onSelect(lineAnchor(line))}
      aria-label={label}
      className="group -mx-1 inline-flex items-center gap-2 rounded-sm px-1 text-left font-medium text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      {line.name}
      {pin}
      <MessageSquarePlus
        aria-hidden
        className="h-3.5 w-3.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
      />
    </button>
  )
}

interface Props {
  lines: QuoteLine[]
  currency: string
  label?: string
  displayOptions?: QuoteDisplayOptions
  hideWhenEmpty?: boolean
  /** Comment count per line, keyed by the line's anchor (its name). */
  commentCountByLine?: Record<string, number>
  onLineSelect?: (lineName: string) => void
  selectedLine?: string
}

export const QuoteLinesTable = ({
  lines,
  currency,
  label = 'Pricing',
  displayOptions,
  hideWhenEmpty,
  commentCountByLine,
  onLineSelect,
  selectedLine,
}: Props) => {
  const shown = displayedLines(lines, displayOptions)
  if (shown.length === 0 && hideWhenEmpty) return null

  return (
    <section className="mb-8">
      <h3 className="border-b border-border pb-3 text-md font-medium text-foreground">{label}</h3>
      {shown.length === 0 ? (
        <div className="p-8 text-center text-muted-foreground">
          No pricing components configured
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full min-w-[320px]">
            <thead className="bg-muted/40">
              <tr>
                <th className={TH}>Name</th>
                <th className={cn(TH, 'hidden sm:table-cell')}>Billing Period</th>
                <th className={cn(TH, 'hidden md:table-cell')}>Fee Type</th>
                <th className={TH}>Price</th>
              </tr>
            </thead>
            <tbody>
              {shown.map(line => {
                const selected = selectedLine !== undefined && selectedLine === lineAnchor(line)
                return (
                  <tr
                    key={line.id}
                    className={cn(
                      'border-b border-border last:border-b-0',
                      selected && 'bg-muted ring-1 ring-inset ring-brand'
                    )}
                  >
                    <td className="px-3 py-3 align-top text-sm sm:px-4">
                      <LineName
                        line={line}
                        commentCount={commentCountByLine?.[lineAnchor(line)] ?? 0}
                        onSelect={onLineSelect}
                      />
                      <div className="mt-0.5 text-xs text-muted-foreground sm:hidden">
                        {formatSubscriptionFeeBillingPeriod(line.period)}
                      </div>
                      {line.description && (
                        <div className="mt-0.5 whitespace-pre-line text-xs text-muted-foreground">
                          {line.description}
                        </div>
                      )}
                    </td>
                    <td className="hidden px-4 py-3 align-top text-sm text-muted-foreground sm:table-cell">
                      {formatSubscriptionFeeBillingPeriod(line.period)}
                    </td>
                    <td className="hidden px-4 py-3 align-top text-sm text-muted-foreground md:table-cell">
                      {(line.fee?.fee.case && FEE_TYPE_LABELS[line.fee.fee.case]) || 'N/A'}
                    </td>
                    <td className="px-3 py-3 align-top text-sm text-muted-foreground sm:px-4">
                      <LinePrice
                        line={line}
                        currency={currency}
                        showListPrice={Boolean(displayOptions?.showListPrice)}
                      />
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}

const TH =
  'px-3 py-3 text-left text-xs font-medium uppercase tracking-wider text-muted-foreground sm:px-4'
