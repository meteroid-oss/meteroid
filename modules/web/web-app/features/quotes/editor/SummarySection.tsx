import { useFormContext, useWatch } from 'react-hook-form'

import { DealSummary } from '@/features/quotes/editor/DealSummary'
import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { PAYMENT_LABELS } from '@/features/quotes/editor/TermsSection'
import { useQuoteTotals } from '@/features/quotes/editor/useQuoteTotals'
import { formatDefaultNetTerms } from '@/lib/mapping/priceToSubscriptionFee'
import { parseAndFormatDate } from '@/utils/date'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { CreateQuote } from '@/rpc/api/quotes/v1/models_pb'
import type { ReactNode } from 'react'

const ACTIVATION_LABELS: Record<QuoteFormValues['activation_condition'], string> = {
  ON_START: 'On start date',
  ON_CHECKOUT: 'On checkout',
  MANUAL: 'Manual',
}

const formatDate = (date?: Date) => (date ? parseAndFormatDate(date.toISOString()) : undefined)

const Fact = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex items-baseline gap-4 text-[13px]">
    <dt className="w-32 shrink-0 text-muted-foreground">{label}</dt>
    <dd className="min-w-0 truncate text-foreground">{children}</dd>
  </div>
)

const Quiet = ({ children }: { children: ReactNode }) => (
  <span className="text-muted-foreground">{children}</span>
)

interface SummarySectionProps {
  customerName?: string
  planName?: string
  currency?: string
  netTermsDefault?: { netTerms: number; sellsCredit: boolean }
  /** The request of the totals; undefined when the pricing can't be expressed as one. */
  buildTotalsRequest: (values: Partial<QuoteFormValues>) => CreateQuote | undefined
  /** Why the totals can't be requested yet, in words for the user. */
  totalsBlockedReason?: string
  /** What the request needs is still loading. */
  totalsLoading: boolean
}

/** The deal at a glance before it goes out: its key terms, and what it comes to. */
export const SummarySection = ({
  customerName,
  planName,
  currency,
  netTermsDefault,
  buildTotalsRequest,
  totalsBlockedReason,
  totalsLoading,
}: SummarySectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const values = useWatch({ control }) as Partial<QuoteFormValues>
  // The same request as the preview's: served from the same cache.
  const request = totalsBlockedReason || totalsLoading ? undefined : buildTotalsRequest(values)
  const totals = useQuoteTotals(request)

  const netTerms =
    values.net_terms !== undefined && values.net_terms !== null && `${values.net_terms}` !== ''
      ? `${values.net_terms} days`
      : netTermsDefault
        ? formatDefaultNetTerms(netTermsDefault)
        : undefined

  return (
    <EditorSection id="quote-summary" title="Summary">
      <dl className="flex flex-col gap-2.5">
        <Fact label="Customer">{customerName ?? <Quiet>None</Quiet>}</Fact>
        <Fact label="Plan">{planName ?? <Quiet>None</Quiet>}</Fact>
        <Fact label="Currency">{currency ?? <Quiet>From the plan</Quiet>}</Fact>
        <Fact label="Quote expires">
          {formatDate(values.expires_at) ?? <Quiet>No expiry</Quiet>}
        </Fact>
        <Fact label="Subscription">
          {values.create_subscription_on_acceptance
            ? 'Created automatically when signed'
            : 'Created manually afterwards'}
        </Fact>
        <Fact label="Starts">
          {formatDate(values.start_date) ?? (
            <Quiet>
              {values.create_subscription_on_acceptance ? 'On signature' : 'When you create it'}
            </Quiet>
          )}
        </Fact>
        <Fact label="Ends">{formatDate(values.end_date) ?? <Quiet>No end date</Quiet>}</Fact>
        <Fact label="Activation">
          {values.activation_condition ? ACTIVATION_LABELS[values.activation_condition] : '—'}
        </Fact>
        <Fact label="Payment">
          {values.payment_methods_type ? PAYMENT_LABELS[values.payment_methods_type] : '—'}
          {netTerms && <Quiet> · net {netTerms}</Quiet>}
        </Fact>
      </dl>

      <DealSummary
        className="shadow-none"
        totals={totals.totals}
        isLoading={totals.isLoading}
        isUpdating={totals.isUpdating}
        isError={totals.isError}
        pendingReason={
          totalsBlockedReason ?? (totalsLoading || request ? undefined : 'Unavailable')
        }
      />
    </EditorSection>
  )
}
