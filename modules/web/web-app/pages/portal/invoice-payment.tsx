import { Skeleton } from '@md/ui'
import { AlertCircle } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import {
  consumeHostedPreAttempt,
  consumeHostedReturn,
  hostedReturnErrorMessage,
} from '@/features/checkout/utils/hostedReturn'
import InvoicePaymentFlow from '@/features/invoice-payment/InvoicePaymentFlow'
import { useQuery } from '@/lib/connectrpc'
import {
  InvoicePaymentStatus,
  Transaction_PaymentStatusEnum,
} from '@/rpc/api/invoices/v1/models_pb'
import { getInvoicePayment } from '@/rpc/portal/invoice/v1/invoice-PortalInvoiceService_connectquery'
import { useTypedParams } from '@/utils/params'
import { useForceTheme } from 'providers/ThemeProvider'

// After an `ok` hosted-flow return the charge is created backend-side —
// by the `billing_requests.fulfilled` webhook for GoCardless (which can lag
// the redirect, or fail), by the return handler itself for Stancer; poll until
// the charge reaches a terminal state, capped so a charge that never lands
// doesn't poll forever.
const HOSTED_POLL_MS = 3000
const HOSTED_POLL_TIMEOUT_MS = 5 * 60 * 1000

export const PortalInvoicePayment = () => {
  useForceTheme('light')

  const invoiceId = useTypedParams<{ invoiceId: string }>().invoiceId

  // Read the hosted-flow return outcome exactly once (it strips the params so a
  // reload/Back doesn't replay it). `ok` = the mandate/card is saved and the
  // charge is created backend-side, so we show "processing" and poll until it
  // resolves. Anything else (abandoned / failed / Stancer payment_failed or
  // processing) surfaces as an error and leaves the payment form available.
  // Browser "back" never hits our return handler, so there's no param and the
  // form just shows — no false "processing". Lazy initializer so it runs
  // exactly once (a re-render must not re-read and null it out — the params
  // are stripped on the first call).
  const [hostedRet] = useState(() => consumeHostedReturn())
  // 'processing' → awaiting the backend-created charge (readonly view + poll);
  // 'failed' → the charge failed: back to the pay form with an error banner;
  // 'timed_out' → nothing landed within the cap: readonly "check back later".
  const [gcPhase, setGcPhase] = useState<'processing' | 'failed' | 'timed_out' | null>(
    hostedRet?.status === 'ok' ? 'processing' : null
  )
  // Transactions already FAILED the first time we see the invoice belong to
  // earlier attempts and must not resolve this return as a failure.
  const staleFailedTxIdsRef = useRef<Set<string> | null>(null)

  const invoicePaymentQuery = useQuery(
    getInvoicePayment,
    { invoiceId },
    {
      refetchInterval: query => {
        if (gcPhase !== 'processing') return false
        const inv = query.state.data?.invoice?.invoice
        // Keep polling while a transaction is merely PENDING — it can still
        // flip to FAILED; only a paid/processing invoice is truly resolved.
        const resolved =
          inv?.paymentStatus === InvoicePaymentStatus.PAID ||
          inv?.paymentStatus === InvoicePaymentStatus.PROCESSING
        return resolved ? false : HOSTED_POLL_MS
      },
    }
  )

  const polledInvoice = invoicePaymentQuery.data?.invoice?.invoice

  useEffect(() => {
    if (gcPhase !== 'processing' || !polledInvoice) return

    const transactions = polledInvoice.transactions ?? []
    let staleFailedTxIds = staleFailedTxIdsRef.current
    if (staleFailedTxIds === null) {
      // Prefer the snapshot captured *before* leaving for the hosted flow: it
      // can't include the new charge, so a charge that fails before this first
      // poll resolves is still correctly seen as newly failed. Fall back to the
      // first-poll failures when there's no snapshot (e.g. a reloaded tab).
      staleFailedTxIds =
        (invoiceId ? consumeHostedPreAttempt(invoiceId) : null) ??
        new Set(
          transactions.filter(t => t.status === Transaction_PaymentStatusEnum.FAILED).map(t => t.id)
        )
      staleFailedTxIdsRef.current = staleFailedTxIds
    }

    if (
      polledInvoice.paymentStatus === InvoicePaymentStatus.PAID ||
      polledInvoice.paymentStatus === InvoicePaymentStatus.PROCESSING
    ) {
      setGcPhase(null)
      return
    }

    const newlyFailed = transactions.some(
      t => t.status === Transaction_PaymentStatusEnum.FAILED && !staleFailedTxIds.has(t.id)
    )
    if (newlyFailed) {
      setGcPhase('failed')
    }
  }, [gcPhase, polledInvoice])

  useEffect(() => {
    if (gcPhase !== 'processing') return
    const timer = setTimeout(() => setGcPhase('timed_out'), HOSTED_POLL_TIMEOUT_MS)
    return () => clearTimeout(timer)
  }, [gcPhase])

  // 'timed_out' keeps the readonly "payment in progress / check back later"
  // view (poll stopped) rather than re-offering the form — the charge may
  // still land and paying again could double-charge.
  const hostedProcessing = gcPhase === 'processing' || gcPhase === 'timed_out'
  const hostedError =
    hostedRet && hostedRet.status !== 'ok'
      ? hostedReturnErrorMessage(hostedRet)
      : gcPhase === 'failed'
        ? hostedRet?.provider === 'stancer'
          ? 'Your card payment could not be completed. Please try again or use a different payment method.'
          : 'Your direct debit payment could not be completed. Please try again or use a different payment method.'
        : null

  const data = invoicePaymentQuery.data?.invoice
  const error = invoicePaymentQuery.error
  const isLoading = invoicePaymentQuery.isLoading

  if (error) {
    return (
      <div className="min-h-screen w-full bg-[#00000002]">
        <div className="flex flex-col items-center justify-center min-h-screen max-w-md mx-auto px-6 py-12 text-center">
          <AlertCircle className="h-8 w-8 text-muted-foreground mb-4" />
          <h2 className="text-md font-semibold text-gray-800 mb-2">Something went wrong</h2>
          <p className="text-gray-800 text-sm">
            There may be a connection issue, your session might be expired or completed, or our
            payment system is temporarily unavailable
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-[#00000002]">
      <div className="flex flex-col gap-4 h-full">
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <InvoicePaymentFlow
            invoicePaymentData={data}
            hostedProcessing={hostedProcessing}
            hostedError={hostedError}
          />
        )}
      </div>
    </div>
  )
}
