import { ReactNode } from 'react'

import { Quote, QuoteRecurringTotal, QuoteTotals } from '@/rpc/api/quotes/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { formatDateOnly, parseDateField } from '@/utils/date'
import { formatCurrencyNoRounding } from '@/utils/numbers'

const PERIOD_UNIT: Partial<Record<SubscriptionFeeBillingPeriod, string>> = {
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'year',
}

export const formatPeriodAmount = (
  amount: string,
  period: SubscriptionFeeBillingPeriod,
  currency: string
) => {
  const money = formatCurrencyNoRounding(amount, currency)
  const unit = PERIOD_UNIT[period]
  return unit ? `${money} / ${unit}` : `${money} one-time`
}

const isZero = (amount: string) => Number(amount) === 0

/** Recurring entries worth showing: a line free at list price says nothing. */
export const visibleRecurring = (totals: QuoteTotals) =>
  totals.recurring.filter(r => !(isZero(r.amount) && isZero(r.listAmount)))

/** The first invoice subtotal, unless it is zero (e.g. usage billed in arrears). */
export const firstInvoiceShown = (totals: QuoteTotals) =>
  totals.firstInvoiceSubtotal !== undefined && !isZero(totals.firstInvoiceSubtotal)
    ? totals.firstInvoiceSubtotal
    : undefined

/** "€2,235.00 / month + €1,000.00 / year + usage", "Usage-based", or undefined. */
export const recurringSummary = (totals?: QuoteTotals) => {
  if (!totals) return undefined
  const parts = visibleRecurring(totals).map(r =>
    formatPeriodAmount(r.amount, r.period, totals.currency)
  )
  if (totals.hasUsage) parts.push(parts.length > 0 ? 'usage' : 'Usage-based')
  return parts.length > 0 ? parts.join(' + ') : undefined
}

/** "12 months" for a whole number of months, else "until Mar 1, 2027"; undefined without an end. */
export const termSummary = (quote?: Quote) => {
  if (!quote?.endDate) return undefined
  if (quote.startDate) {
    const start = parseDateField(quote.startDate)
    const end = parseDateField(quote.endDate)
    const months =
      (end.getUTCFullYear() - start.getUTCFullYear()) * 12 + end.getUTCMonth() - start.getUTCMonth()
    if (months > 0 && end.getUTCDate() === start.getUTCDate()) {
      return months === 1 ? '1 month' : `${months} months`
    }
  }
  return `until ${formatDateOnly(quote.endDate)}`
}

const isDiscounted = (total: QuoteRecurringTotal) => Number(total.amount) < Number(total.listAmount)

const Row = ({
  label,
  value,
  emphasis = false,
}: {
  label: string
  value: ReactNode
  emphasis?: boolean
}) => (
  <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b border-border px-3 py-3 last:border-b-0 sm:px-4">
    <dt className={emphasis ? 'font-semibold text-foreground' : 'text-muted-foreground'}>
      {label}
    </dt>
    <dd
      className={
        emphasis ? 'text-right font-semibold text-foreground' : 'text-right text-foreground'
      }
    >
      {value}
    </dd>
  </div>
)

export const QuoteTotalsRows = ({
  totals,
  showListPrice,
}: {
  totals: QuoteTotals
  showListPrice: boolean
}) => {
  const recurring = visibleRecurring(totals)
  // Usage alone is already explained by its line: no totals without a fixed amount.
  if (recurring.length === 0) return null
  const multiplePeriods = recurring.length > 1
  const usageOn = recurring.findIndex(r => PERIOD_UNIT[r.period] !== undefined)
  const firstInvoice = firstInvoiceShown(totals)

  return (
    <dl className="mb-8 rounded-lg border border-border text-sm">
      {recurring.map((total, index) => {
        const discounted = isDiscounted(total)
        const unit = PERIOD_UNIT[total.period]
        return (
          <Row
            key={total.period}
            emphasis
            label={
              !unit ? 'One-time' : multiplePeriods ? `Recurring, per ${unit}` : 'Recurring total'
            }
            value={
              <span className="inline-flex flex-wrap items-center justify-end gap-x-2 gap-y-1">
                {discounted && showListPrice && (
                  <span className="text-xs font-normal text-muted-foreground line-through">
                    {formatPeriodAmount(total.listAmount, total.period, totals.currency)}
                  </span>
                )}
                <span>{formatPeriodAmount(total.amount, total.period, totals.currency)}</span>
                {totals.hasUsage && index === Math.max(usageOn, 0) && (
                  <span className="font-normal text-muted-foreground">+ usage</span>
                )}
              </span>
            }
          />
        )
      })}
      {firstInvoice !== undefined && (
        <Row
          label="First invoice, before tax"
          value={formatCurrencyNoRounding(firstInvoice, totals.currency)}
        />
      )}
    </dl>
  )
}
