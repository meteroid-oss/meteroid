import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { AlertCircle } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { CheckoutThemePane } from '@/features/checkout/CheckoutThemePane'
import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { BillingInfo } from '@/features/checkout/components/BillingInfo'
import { ReadonlyPaymentView } from '@/features/checkout/components/ReadonlyPaymentView'
import { resolveCheckoutTheme } from '@/features/checkout/resolveCheckoutTheme'
import { completeNextAction } from '@/features/checkout/utils/completeNextAction'
import { hostedReturnUrl, stashHostedPreAttempt } from '@/features/checkout/utils/hostedReturn'
import { getInvoicePaymentAvailability } from '@/features/checkout/utils/paymentAvailability'
import { Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'
import {
  confirmInvoicePayment,
  getInvoicePayment,
  initiateHostedInvoicePayment,
} from '@/rpc/portal/invoice/v1/invoice-PortalInvoiceService_connectquery'
import { formatCurrency } from '@/utils/numbers'

import { BankTransferInfo } from './components/BankTransferInfo'
import { InvoicePdfDownload } from './components/InvoicePdfDownload'
import { InvoiceSummary } from './components/InvoiceSummary'
import { TransactionList } from './components/TransactionList'
import { InvoicePaymentData } from './types'

interface Props extends InvoicePaymentData {
  /** Customer just completed a hosted flow (GoCardless mandate / Stancer card):
   *  the backend attaches the method and charges this invoice, so show
   *  "processing" instead of the pay form. */
  hostedProcessing?: boolean
  /** Non-success hosted-flow return (abandoned/failed/declined): shown above
   *  the still-available pay form so the customer can retry. */
  hostedError?: string | null
}

const InvoicePaymentFlow: React.FC<Props> = ({
  invoicePaymentData,
  hostedProcessing = false,
  hostedError = null,
}) => {
  const [isAddressEditing, setIsAddressEditing] = useState(false)
  const [isResuming, setIsResuming] = useState(false)
  const [resumeError, setResumeError] = useState<string | null>(null)
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const themeConfig = useMemo(
    () =>
      resolveCheckoutTheme({
        brandColor: invoicePaymentData.brandColor,
        themeMode: invoicePaymentData.themeMode,
        roundness: invoicePaymentData.roundness,
        logoUrl: invoicePaymentData.logoUrl,
        tradeName: invoicePaymentData.tradeName,
      }),
    [invoicePaymentData]
  )
  const {
    invoice,
    customer,
    paymentMethods,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  } = invoicePaymentData

  // Transactions already FAILED before the customer leaves for a provider-
  // hosted flow. Snapshotted on departure so the return handler distinguishes a
  // genuinely new charge failure from these pre-existing attempts.
  const preAttemptFailedTxIds = useMemo(
    () =>
      (invoice?.transactions ?? [])
        .filter(t => t.status === Transaction_PaymentStatusEnum.FAILED)
        .map(t => t.id),
    [invoice?.transactions]
  )

  const confirmInvoicePaymentMutation = useMutation(confirmInvoicePayment, {
    onError: error => {
      console.error('Invoice payment confirmation error:', error)
    },
  })

  const initiateHostedInvoicePaymentMutation = useMutation(initiateHostedInvoicePayment)

  // Explicit pay CLICK for hosted-redirect card providers (Stancer): the RPC
  // pre-creates the invoice transaction, mints the capturing intent and
  // returns the redirect we follow. Never called on render.
  const handleHostedInvoicePayment = async (connectionId: string) => {
    if (!invoice?.currency) {
      throw new Error('Currency is not defined')
    }
    stashHostedPreAttempt(invoice.id, preAttemptFailedTxIds)
    const res = await initiateHostedInvoicePaymentMutation.mutateAsync({
      connectionId,
      invoiceId: invoice.id,
      displayedAmount: invoice.amountDue,
      displayedCurrency: invoice.currency,
      returnUrl: hostedReturnUrl(),
    })
    if (!res.nextAction) {
      throw new Error('The payment provider returned no redirect. Please try again.')
    }
    // Redirects to the provider-hosted page; never resolves.
    await completeNextAction(res.nextAction)
  }

  // "Continue payment" for an abandoned hosted attempt: the same RPC resumes
  // the SAME stored intent/redirect (single-intent discipline), so no second
  // capturable intent can be minted.
  const handleResumeHostedPayment = async (connectionId: string) => {
    setResumeError(null)
    setIsResuming(true)
    try {
      await handleHostedInvoicePayment(connectionId)
    } catch (error) {
      console.error('Hosted payment resume error:', error)
      setResumeError(
        error instanceof Error ? error.message : 'Unable to resume the payment. Please try again.'
      )
      setIsResuming(false)
    }
  }

  // Refetch the invoice so a freshly attached method appears in the list.
  const refreshInvoice = useCallback(() => {
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: getInvoicePayment, cardinality: undefined }),
    })
  }, [queryClient])

  const handlePaymentSubmit = async (paymentMethodId: string) => {
    try {
      if (!invoice?.currency) {
        throw new Error('Currency is not defined')
      }

      const res = await confirmInvoicePaymentMutation.mutateAsync({
        displayedAmount: invoice.amountDue,
        displayedCurrency: invoice.currency,
        paymentMethodId,
        invoiceId: invoice.id,
      })

      // If the charge needs 3DS/SCA, complete it before navigating.
      await completeNextAction(res.nextAction)

      const params = new URLSearchParams({
        invoice: invoice.invoiceNumber || '',
        customer: customer?.name || '',
      })
      navigate(`success?${params.toString()}`)
    } catch (error) {
      console.error('Payment submission error:', error)
      throw error // Let the PaymentPanel handle this error
    }
  }

  // NB: the hosted-flow return outcome is read ONCE by the parent page and
  // passed in as `hostedProcessing` / `hostedError`. The invoice charge itself
  // is created backend-side (GoCardless: the `billing_requests.fulfilled`
  // webhook; Stancer: the return handler), so on `ok` we only show
  // "processing" and the page polls until the transaction appears — we never
  // confirm the payment from the return.

  if (!invoice || !customer) {
    return <div className="p-8 text-center">Loading invoice payment information...</div>
  }

  const baseAvailability = getInvoicePaymentAvailability({
    invoiceStatus: invoice.status,
    paymentStatus: invoice.paymentStatus,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
    hasTransactions: (invoice.transactions?.length ?? 0) > 0,
    transactions: invoice.transactions,
  })

  // Just returned from a hosted flow: the charge is created backend-side, so
  // the pending transaction may not be visible yet (the page polls for it —
  // see the invoice-payment page). Until it lands, show the "processing" state
  // instead of the payment form, so the customer isn't invited to pay again on
  // a payment they just authorised. Once the transaction appears, this is
  // already `readonly` on its own.
  // The resumable state is coerced too: right after authorising on the hosted
  // page the Pending tx still carries its marker, and inviting the customer to
  // "continue" a payment they just completed would be wrong.
  const paymentAvailability =
    hostedProcessing &&
    (baseAvailability.type === 'payment_form' ||
      baseAvailability.type === 'resumable_hosted_payment')
      ? ({ type: 'readonly', reason: 'pending_payment', displayTransactions: true } as const)
      : baseAvailability

  return (
    <div className="w-full flex flex-col lg:flex-row min-h-screen lg:h-screen lg:overflow-hidden">
      {/* Left panel - Invoice summary */}
      <div className="flex flex-col bg-background-gray gap-5 w-full lg:w-3/5 px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 xl:pb-20 border-b lg:border-b-0 lg:border-r border-border-regular lg:h-screen lg:overflow-auto">
        <div className="w-full">
          <InvoiceSummary invoicePaymentData={invoicePaymentData} />
        </div>
      </div>
      {/* Right panel - Payment form */}
      <CheckoutThemePane
        config={themeConfig}
        className="w-full lg:w-2/5 flex flex-col px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 lg:h-screen lg:overflow-auto shadow-md"
      >
        <div className="w-full max-w-[440px] mx-auto lg:mx-0 space-y-8">
          {/* A "processing" return renders as the readonly Payment In Progress
                view below. A non-success return (abandoned/failed) surfaces here
                as a banner, above the pay form which stays available for retry. */}
          {hostedError && (
            <div
              className="rounded-md border px-4 py-3 text-sm flex items-start"
              style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
            >
              <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
              <span>{hostedError}</span>
            </div>
          )}

          {/* Billing information */}
          <BillingInfo
            customer={customer}
            isEditing={isAddressEditing}
            setIsEditing={setIsAddressEditing}
          />

          {/* Invoice PDF Download */}
          <InvoicePdfDownload
            invoiceId={invoice.id}
            invoiceNumber={invoice.invoiceNumber}
            documentSharingKey={invoice.documentSharingKey}
            pdfDocumentId={invoice.pdfDocumentId}
          />

          {/* Render based on payment availability */}
          {paymentAvailability.type === 'readonly' && (
            <>
              <ReadonlyPaymentView reason={paymentAvailability.reason} />
              {paymentAvailability.displayTransactions && invoice.transactions && (
                <TransactionList transactions={invoice.transactions} currency={invoice.currency} />
              )}
            </>
          )}

          {paymentAvailability.type === 'resumable_hosted_payment' && (
            <>
              <ReadonlyPaymentView
                reason="pending_payment"
                title="Payment Not Completed"
                message="You started a payment for this invoice but did not finish it. You can continue where you left off."
              />

              {resumeError && (
                <div
                  className="p-3 rounded-lg text-sm flex items-start"
                  style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
                >
                  <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
                  <span>{resumeError}</span>
                </div>
              )}

              <button
                type="button"
                onClick={() => handleResumeHostedPayment(paymentAvailability.connectionId)}
                disabled={isResuming}
                className="w-full py-3 rounded-lg transition-all font-medium disabled:cursor-not-allowed disabled:opacity-60 hover:opacity-90"
                style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
              >
                {isResuming ? (
                  <div className="flex items-center justify-center">
                    <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin mr-2"></div>
                    Processing...
                  </div>
                ) : (
                  `Continue payment · ${formatCurrency(Number(invoice.amountDue) || 0, invoice.currency)}`
                )}
              </button>

              {paymentAvailability.displayTransactions && invoice.transactions && (
                <TransactionList transactions={invoice.transactions} currency={invoice.currency} />
              )}
            </>
          )}

          {paymentAvailability.type === 'bank_only' && (
            <BankTransferInfo
              bankAccount={paymentAvailability.bankAccount}
              invoiceNumber={invoice.invoiceNumber}
              customerName={customer?.name}
            />
          )}

          {paymentAvailability.type === 'payment_form' && (
            <>
              {/* Show payment panel if card or DD available */}
              {(paymentAvailability.cardConnectionId ||
                paymentAvailability.directDebitConnectionId) && (
                <PaymentPanel
                  customer={customer}
                  paymentMethods={paymentMethods || []}
                  currency={invoice.currency}
                  totalAmount={formatCurrency(Number(invoice.amountDue) || 0, invoice.currency)}
                  onPaymentSubmit={handlePaymentSubmit}
                  onPaymentMethodAttached={refreshInvoice}
                  cardConnectionId={paymentAvailability.cardConnectionId}
                  directDebitConnectionId={paymentAvailability.directDebitConnectionId}
                  invoiceId={invoice.id}
                  preAttemptFailedTxIds={preAttemptFailedTxIds}
                  onHostedInvoicePayment={handleHostedInvoicePayment}
                  themeConfig={themeConfig}
                />
              )}

              {/* A method that couldn't be set up (e.g. missing customer
                    details) is silently dropped here: the reason is a merchant
                    misconfiguration, not something the paying customer can act
                    on. It's surfaced to the org on the customer details page. */}

              {/* Show bank transfer as alternative if available */}
              {paymentAvailability.bankAccount &&
                !paymentAvailability.cardConnectionId &&
                !paymentAvailability.directDebitConnectionId && (
                  <div className="mt-6">
                    <div
                      className="text-center text-sm mb-4"
                      style={{ color: 'var(--mtp-text-2)' }}
                    >
                      or
                    </div>
                    <BankTransferInfo
                      bankAccount={paymentAvailability.bankAccount}
                      invoiceNumber={invoice.invoiceNumber}
                      customerName={customer?.name}
                    />
                  </div>
                )}
            </>
          )}
        </div>
      </CheckoutThemePane>
    </div>
  )
}

export default InvoicePaymentFlow
