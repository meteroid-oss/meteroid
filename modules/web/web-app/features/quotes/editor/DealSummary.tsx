import { Card, CardContent, CardHeader, CardTitle, Skeleton } from '@md/ui'
import { Loader2 } from 'lucide-react'

import { firstInvoiceShown, visibleRecurring } from '@/features/quotes/QuoteViewTotals'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { formatCurrencyNoRounding } from '@/utils/numbers'

import type { QuoteTotals } from '@/rpc/api/quotes/v1/models_pb'
import type { ReactNode } from 'react'

const PERIOD_LABELS: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'One-time',
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'Recurring, per month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'Recurring, per quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'Recurring, per half-year',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'Recurring, per year',
}

const Row = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex items-baseline justify-between gap-3 text-sm">
    <span className="text-muted-foreground">{label}</span>
    <span className="text-right tabular-nums">{children}</span>
  </div>
)

interface DealSummaryProps {
  totals?: QuoteTotals
  isLoading: boolean
  isUpdating: boolean
  isError: boolean
  /** Why no totals can be computed yet, when the editor can't send its state. */
  pendingReason?: string
  className?: string
}

export const DealSummary = ({
  totals,
  isLoading,
  isUpdating,
  isError,
  pendingReason,
  className,
}: DealSummaryProps) => {
  const money = (amount: string) =>
    totals ? formatCurrencyNoRounding(amount, totals.currency) : amount

  const body = () => {
    if (pendingReason) {
      return <p className="text-sm text-muted-foreground">{pendingReason}</p>
    }
    if (totals) {
      const recurring = visibleRecurring(totals)
      const firstInvoice = firstInvoiceShown(totals)
      return (
        <div className="flex flex-col gap-2">
          {recurring.length === 0 && !totals.hasUsage && (
            <p className="text-sm text-muted-foreground">—</p>
          )}
          {recurring.map(total => {
            const discounted = Number(total.listAmount) !== Number(total.amount)
            return (
              <Row key={total.period} label={PERIOD_LABELS[total.period] ?? 'Recurring'}>
                {discounted && (
                  <span className="mr-1.5 text-xs text-muted-foreground line-through">
                    {money(total.listAmount)}
                  </span>
                )}
                {money(total.amount)}
              </Row>
            )
          })}
          {totals.hasUsage && <Row label="Usage">On consumption</Row>}
          {firstInvoice !== undefined && (
            <Row label="First invoice">
              {money(firstInvoice)}
              <span className="ml-1 text-xs text-muted-foreground">+ tax</span>
            </Row>
          )}
        </div>
      )
    }
    if (isError && !isLoading) {
      return <p className="text-sm text-muted-foreground">Unavailable</p>
    }
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        <Skeleton height={16} />
        <Skeleton height={16} width="70%" />
        <Skeleton height={16} width="85%" />
      </div>
    )
  }

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 p-4 pb-3">
        <CardTitle className="text-sm">Deal summary</CardTitle>
        {isUpdating && totals && (
          <Loader2
            className="h-3.5 w-3.5 animate-spin text-muted-foreground"
            aria-label="Updating"
          />
        )}
      </CardHeader>
      <CardContent className="p-4 pt-0" aria-live="polite">
        {body()}
      </CardContent>
    </Card>
  )
}
