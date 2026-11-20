import { Card, CardContent, CardHeader, CardTitle } from '@md/ui'
import { Maximize2 } from 'lucide-react'
import { useDeferredValue } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import { QuoteView } from '@/features/quotes/QuoteView'
import { DealSummary } from '@/features/quotes/editor/DealSummary'
import { useQuoteTotals } from '@/features/quotes/editor/useQuoteTotals'

import type { QuoteLine } from '@/features/quotes/detail/QuoteLinesTable'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import type { CreateQuote, DetailedQuote } from '@/rpc/api/quotes/v1/models_pb'

const PREVIEW_SCALE = 0.36

const setInert = (element: HTMLDivElement | null) => element?.setAttribute('inert', '')

type QuotePreview = { quote: DetailedQuote; lines: QuoteLine[] }

/** The whole document, with the totals of the same request as the rail's (served from cache). */
export const QuoteFullPreview = ({
  preview,
  totalsRequest,
  minimumCommitment,
}: {
  preview: QuotePreview
  totalsRequest?: CreateQuote
  minimumCommitment?: MinimumCommitment
}) => {
  const { totals } = useQuoteTotals(totalsRequest)
  return (
    <QuoteView
      quote={preview.quote}
      mode="preview"
      subscriptionComponents={preview.lines}
      minimumCommitment={minimumCommitment}
      totals={totals}
    />
  )
}

interface QuoteEditorRailProps {
  buildPreview: (values: Partial<QuoteFormValues>) => QuotePreview
  /** The request of the totals; undefined when the pricing can't be expressed as one. */
  buildTotalsRequest: (values: Partial<QuoteFormValues>) => CreateQuote | undefined
  /** Why the totals can't be requested yet, in words for the user. */
  totalsBlockedReason?: string
  /** What the request needs is still loading. */
  totalsLoading: boolean
  minimumCommitment?: MinimumCommitment
  canOpenPreview: boolean
  onOpenPreview: () => void
  /** Totals need a customer: a template has none to price for. */
  showSummary?: boolean
}

export const QuoteEditorRail = ({
  buildPreview,
  buildTotalsRequest,
  totalsBlockedReason,
  totalsLoading,
  minimumCommitment,
  canOpenPreview,
  onOpenPreview,
  showSummary = true,
}: QuoteEditorRailProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const values = useWatch({ control }) as Partial<QuoteFormValues>
  const deferredValues = useDeferredValue(values)

  const preview = buildPreview(deferredValues)
  const request =
    !showSummary || totalsBlockedReason || totalsLoading ? undefined : buildTotalsRequest(values)
  const totals = useQuoteTotals(request)

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 p-4 pb-3">
          <CardTitle className="text-sm">Preview</CardTitle>
          {canOpenPreview && (
            <button
              type="button"
              onClick={onOpenPreview}
              className="flex shrink-0 items-center gap-1 text-xs text-brand hover:underline"
            >
              <Maximize2 className="h-3 w-3" />
              Full preview
            </button>
          )}
        </CardHeader>
        <CardContent className="p-4 pt-0">
          <div className="relative h-72 overflow-hidden rounded-md border border-border bg-background">
            <div
              ref={setInert}
              aria-hidden
              className="pointer-events-none absolute left-0 top-0 origin-top-left"
              style={{ width: `${100 / PREVIEW_SCALE}%`, transform: `scale(${PREVIEW_SCALE})` }}
            >
              <QuoteView
                quote={preview.quote}
                mode="preview"
                subscriptionComponents={preview.lines}
                minimumCommitment={minimumCommitment}
                totals={totals.totals}
              />
            </div>
            {canOpenPreview && (
              <button
                type="button"
                onClick={onOpenPreview}
                className="absolute inset-0 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label="Open the full preview"
              />
            )}
          </div>
        </CardContent>
      </Card>

      {showSummary && (
        <DealSummary
          totals={totals.totals}
          isLoading={totals.isLoading}
          isUpdating={totals.isUpdating}
          isError={totals.isError}
          pendingReason={
            totalsBlockedReason ?? (totalsLoading || request ? undefined : 'Unavailable')
          }
        />
      )}
    </div>
  )
}
