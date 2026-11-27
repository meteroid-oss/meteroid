import { cn } from '@md/ui'
import { useDeferredValue, useLayoutEffect, useRef, useState } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import { QuoteView } from '@/features/quotes/QuoteView'
import { useQuoteTotals } from '@/features/quotes/editor/useQuoteTotals'

import type { QuoteLine } from '@/features/quotes/detail/QuoteLinesTable'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import type { CreateQuote, DetailedQuote } from '@/rpc/api/quotes/v1/models_pb'

/** The document's own width; the pane scales it down to fit, never up. */
const PAPER_WIDTH = 816
/** The sheet: a step lighter than the pane, a hairline edge, the preview's type (see
 *  `.mt-quote-paper` in main.css), and at least a page tall (A4's height for that width,
 *  816 × √2), so a short quote still reads as a sheet. */
/** Lifted off the canvas: a hairline shadow edge and a soft drop. */
const SHEET_SHADOW = 'shadow-[0_0_0.5px_rgba(0,0,0,0.12),0_24px_12px_rgba(0,0,0,0.08)]'

const PAPER_CLASS =
  'mt-quote-paper min-h-[1154px] rounded-none border-[0.5px] border-black/10 dark:border-white/10 dark:bg-[#2e2e2e]'

const setInert = (element: HTMLDivElement | null) => element?.setAttribute('inert', '')

type QuotePreview = { quote: DetailedQuote; lines: QuoteLine[] }

/** The whole document, with the totals of the same request as the side preview's (cached). */
export const QuoteFullPreview = ({
  preview,
  totalsRequest,
  quoteId,
  minimumCommitment,
}: {
  preview: QuotePreview
  totalsRequest?: CreateQuote
  quoteId?: string
  minimumCommitment?: MinimumCommitment
}) => {
  const { totals, priceSchedule } = useQuoteTotals(totalsRequest, quoteId)
  return (
    <QuoteView
      quote={{ ...preview.quote, priceSchedule }}
      mode="preview"
      subscriptionComponents={preview.lines}
      minimumCommitment={minimumCommitment}
      totals={totals}
    />
  )
}

/** Scales the document to the pane's width, and keeps the pane as tall as the scaled document. */
const useFitToWidth = () => {
  const frame = useRef<HTMLDivElement>(null)
  const paper = useRef<HTMLDivElement>(null)
  const [fit, setFit] = useState({ scale: 0.6, height: 0 })

  useLayoutEffect(() => {
    const measure = () => {
      if (!frame.current || !paper.current) return
      const scale = Math.min(1, frame.current.clientWidth / PAPER_WIDTH)
      const height = paper.current.offsetHeight * scale
      setFit(prev => (prev.scale === scale && prev.height === height ? prev : { scale, height }))
    }
    const observer = new ResizeObserver(measure)
    if (frame.current) observer.observe(frame.current)
    if (paper.current) observer.observe(paper.current)
    measure()
    return () => observer.disconnect()
  }, [])

  return { frame, paper, ...fit }
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
  withTotals?: boolean
  /** The draft edited, whose saved phases the totals price. */
  quoteId?: string
}

/** The editor's side pane: the document as it stands, live, with its totals. */
export const QuoteEditorRail = ({
  buildPreview,
  buildTotalsRequest,
  totalsBlockedReason,
  totalsLoading,
  minimumCommitment,
  canOpenPreview,
  onOpenPreview,
  withTotals = true,
  quoteId,
}: QuoteEditorRailProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const values = useWatch({ control }) as Partial<QuoteFormValues>
  const deferredValues = useDeferredValue(values)

  const preview = buildPreview(deferredValues)
  const request =
    !withTotals || totalsBlockedReason || totalsLoading ? undefined : buildTotalsRequest(values)
  const totals = useQuoteTotals(request, quoteId)
  const { frame, paper, scale, height } = useFitToWidth()

  return (
    // my-auto: centred in the pane's height while it fits, scrolling from the top once it doesn't.
    <div className="mx-auto my-auto flex w-full max-w-[680px] flex-col gap-4">
      <div ref={frame} className="relative w-full" style={{ height: height || undefined }}>
        <div
          ref={el => {
            paper.current = el
            setInert(el)
          }}
          aria-hidden
          className={cn('pointer-events-none absolute left-0 top-0 origin-top-left', SHEET_SHADOW)}
          style={{ width: PAPER_WIDTH, transform: `scale(${scale})` }}
        >
          <QuoteView
            quote={{ ...preview.quote, priceSchedule: totals.priceSchedule }}
            mode="preview"
            subscriptionComponents={preview.lines}
            minimumCommitment={minimumCommitment}
            totals={totals.totals}
            className={PAPER_CLASS}
          />
        </div>
        {canOpenPreview && (
          <button
            type="button"
            onClick={onOpenPreview}
            className="absolute inset-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Open the preview as the customer sees it"
          />
        )}
      </div>
    </div>
  )
}
