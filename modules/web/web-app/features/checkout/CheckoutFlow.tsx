import { Code, ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { AlertCircle } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'

import { CheckoutThemePane } from '@/features/checkout/CheckoutThemePane'
import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { ReadonlyPaymentView } from '@/features/checkout/components/ReadonlyPaymentView'
import { resolveCheckoutTheme } from '@/features/checkout/resolveCheckoutTheme'
import { hasCompleteBillingInformation } from '@/features/checkout/utils/billingInfo'
import { completeNextAction } from '@/features/checkout/utils/completeNextAction'
import {
  consumeHostedReturn,
  hostedReturnErrorMessage,
  hostedReturnUrl,
} from '@/features/checkout/utils/hostedReturn'
import { getCheckoutPaymentAvailability } from '@/features/checkout/utils/paymentAvailability'
import { BillingInfo } from '@/features/customers/components/BillingInfo'
import { BankTransferInfo } from '@/features/invoice-payment/components/BankTransferInfo'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  confirmCheckout,
  getCheckout,
  initiateHostedCheckout,
} from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { CheckoutType } from '@/rpc/portal/checkout/v1/checkout_pb'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

import { SubscriptionSummary } from './components/SubscriptionSummary'
import { CheckoutFlowProps } from './types'

// After a hosted flow returns `ok`, the backend materializes the subscription
// (GoCardless: webhook, which can lag the redirect; Stancer: the return
// handler) — poll the checkout until the session reports completed.
const HOSTED_ACTIVATION_POLL_MS = 3000
const HOSTED_ACTIVATION_TIMEOUT_MS = 2 * 60 * 1000

const CheckoutFlow: React.FC<CheckoutFlowProps> = ({
  checkoutData: initialCheckoutData,
  checkoutType,
  planChangeContext,
  addonPurchaseContext,
}) => {
  const [isAddressEditing, setIsAddressEditing] = useState(
    initialCheckoutData.requireBillingInformation &&
      !hasCompleteBillingInformation(initialCheckoutData.customer)
  )
  const [couponCode, setCouponCode] = useState('')
  const [couponError, setCouponError] = useState<string | undefined>(undefined)
  const [isApplyingCoupon, setIsApplyingCoupon] = useState(false)
  const [checkoutData, setCheckoutData] = useState<Checkout>(initialCheckoutData)
  // Hosted-flow round trip (GoCardless mandate / Stancer card): the customer is
  // redirected back here with a gocardless_status / stancer_status param.
  // Lazy initializer so the params are read (and stripped) exactly once — a
  // re-run of the mount effect (StrictMode) must see the same outcome.
  const [hostedReturn] = useState(() => consumeHostedReturn())
  const [hostedError, setHostedError] = useState<string | null>(null)
  // On an `ok` hosted-checkout return the mandate/card is saved and the first
  // payment submitted; the backend finalizes everything. The frontend only
  // observes: 'processing' while polling, 'delayed' past timeout.
  const [hostedReturnPhase, setHostedReturnPhase] = useState<'processing' | 'delayed' | null>(() =>
    hostedReturn?.status === 'ok' ? 'processing' : null
  )
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const returnUrl = searchParams.get('return_url')
  const themeConfig = useMemo(
    () =>
      resolveCheckoutTheme({
        brandColor: initialCheckoutData.brandColor,
        themeMode: initialCheckoutData.themeMode,
        roundness: initialCheckoutData.roundness,
        logoUrl: initialCheckoutData.logoUrl,
        tradeName: initialCheckoutData.tradeName,
      }),
    [initialCheckoutData]
  )
  const {
    subscription,
    customer,
    paymentMethods,
    amountDue,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
    requireBillingInformation,
  } = checkoutData

  const billingBlocksPayment = requireBillingInformation && !hasCompleteBillingInformation(customer)

  const confirmCheckoutMutation = useMutation(confirmCheckout, {
    onError: error => {
      console.error('Checkout confirmation error:', error)
    },
  })

  const initiateHostedCheckoutMutation = useMutation(initiateHostedCheckout, {
    onError: error => {
      console.error('Hosted checkout initiation error:', error)
    },
  })

  const applyCouponMutation = useMutation(getCheckout)

  const navigateToSuccess = useCallback(
    // `processing` for direct debit, where the payment is submitted but settles
    // later — the success page says "you're all set / payment processing" rather
    // than "payment successful".
    (processing = false) => {
      const params = new URLSearchParams({
        plan: subscription?.subscription?.planName || '',
        customer: customer?.name || '',
      })
      if (returnUrl) {
        params.set('return_url', returnUrl)
      }
      if (processing) {
        params.set('status', 'processing')
      }
      navigate(`success?${params.toString()}`)
    },
    [subscription, customer, returnUrl, navigate]
  )

  // Re-pull the checkout (preserving any applied coupon) so a freshly attached
  // payment method appears in the list.
  const refreshCheckout = useCallback(async () => {
    try {
      const response = await applyCouponMutation.mutateAsync(
        couponCode.trim() ? { couponCode: couponCode.trim() } : {}
      )
      if (response.checkout) {
        setCheckoutData(response.checkout)
      }
    } catch {
      // best-effort refresh; keep current data on failure
    }
  }, [applyCouponMutation, couponCode])

  const handleApplyCoupon = async () => {
    const code = couponCode.trim()
    if (!code) return

    setIsApplyingCoupon(true)
    setCouponError(undefined)

    try {
      const response = await applyCouponMutation.mutateAsync({
        couponCode: code,
      })

      if (response.checkout) {
        setCheckoutData(response.checkout)
      }
    } catch (error) {
      if (error instanceof ConnectError) {
        setCouponError(error.message)
      } else {
        setCouponError('Failed to apply coupon')
      }
    } finally {
      setIsApplyingCoupon(false)
    }
  }

  const handleClearCoupon = async () => {
    setCouponCode('')
    setCouponError(undefined)

    try {
      const response = await applyCouponMutation.mutateAsync({})
      if (response.checkout) {
        setCheckoutData(response.checkout)
      }
    } catch {
      setCheckoutData(initialCheckoutData)
    }
  }

  const handlePaymentSubmit = async (paymentMethodId: string) => {
    try {
      setCouponError(undefined)

      if (billingBlocksPayment) {
        setIsAddressEditing(true)
        throw new Error('Please complete your billing information before continuing.')
      }

      if (!subscription?.subscription?.currency) {
        throw new Error('Currency is not defined')
      }

      const res = await confirmCheckoutMutation.mutateAsync({
        displayedAmount: amountDue,
        displayedCurrency: subscription.subscription.currency,
        paymentMethodId,
        couponCode: couponCode.trim() || undefined,
      })

      // If the charge needs 3DS/SCA, complete it before navigating.
      await completeNextAction(res.nextAction)

      navigateToSuccess()
    } catch (error) {
      console.error('Payment submission error:', error)

      if (
        error instanceof ConnectError &&
        error.message.toLowerCase().includes('coupon') &&
        (error.code === Code.NotFound || error.code === Code.InvalidArgument)
      ) {
        // Surface the coupon issue AND keep the panel retryable — resolving
        // here would flip the panel to a false SUCCESS while nothing was
        // charged. Throwing keeps it in ERROR; a retry reuses the already
        // attached method (PaymentPanel remembers it) rather than re-confirming
        // the consumed SetupIntent.
        const message = 'No active coupon found with this code.'
        setCouponError(message)
        throw new Error(message)
      }

      throw error // Let the PaymentPanel handle this error
    }
  }

  // Hosted checkout: ONE explicit customer action. The RPC validates the
  // displayed amount server-side and returns a redirect next_action we follow.
  const handleHostedCheckout = async (connectionId: string) => {
    setCouponError(undefined)

    if (billingBlocksPayment) {
      setIsAddressEditing(true)
      throw new Error('Please complete your billing information before continuing.')
    }

    if (!subscription?.subscription?.currency) {
      throw new Error('Currency is not defined')
    }

    const res = await initiateHostedCheckoutMutation.mutateAsync({
      connectionId,
      displayedAmount: amountDue,
      displayedCurrency: subscription.subscription.currency,
      couponCode: couponCode.trim() || undefined,
      returnUrl: hostedReturnUrl(),
    })

    if (!res.nextAction) {
      throw new Error('The payment provider returned no redirect. Please try again.')
    }

    // Redirects to the provider-hosted page; never resolves.
    await completeNextAction(res.nextAction)
  }

  // Handle the hosted-checkout return once, on mount. On `ok` activation is
  // entirely backend-driven, so we only OBSERVE: poll until the session
  // reports completed, then navigate to success. On any other outcome show
  // the inline error and keep the payment form usable — a card saved by a
  // declined first charge is offered for retry.
  useEffect(() => {
    const ret = hostedReturn
    if (!ret) return

    if (ret.status !== 'ok') {
      setHostedError(hostedReturnErrorMessage(ret))
      return
    }

    let cancelled = false

    const poll = async () => {
      const deadline = Date.now() + HOSTED_ACTIVATION_TIMEOUT_MS
      while (!cancelled && Date.now() < deadline) {
        try {
          const response = await applyCouponMutation.mutateAsync(
            couponCode.trim() ? { couponCode: couponCode.trim() } : {}
          )
          if (cancelled) return
          if (response.checkout) {
            if (
              response.checkout.subscription?.subscription?.status === SubscriptionStatus.ACTIVE
            ) {
              navigateToSuccess(true)
              return
            }
            setCheckoutData(response.checkout)
          }
        } catch (error) {
          if (cancelled) return
          // Once the webhook materializes the subscription the session is
          // completed, and GetCheckout rejects it — that IS the success signal.
          if (
            error instanceof ConnectError &&
            error.message.toLowerCase().includes('already been completed')
          ) {
            navigateToSuccess(true)
            return
          }
          // best-effort refresh; keep polling until the deadline
        }
        await new Promise(resolve => setTimeout(resolve, HOSTED_ACTIVATION_POLL_MS))
      }
      if (!cancelled) {
        setHostedReturnPhase('delayed')
      }
    }
    poll()

    return () => {
      cancelled = true
    }
    // Run once on mount; the return outcome was consumed by the lazy state
    // initializer above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  if (!subscription?.subscription || !customer) {
    return <div className="p-8 text-center">Loading checkout information...</div>
  }

  const paymentAvailability = getCheckoutPaymentAvailability({
    subscriptionStatus: subscription.subscription.status,
    checkoutType,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  })

  return (
    <div className="w-full flex flex-col lg:flex-row min-h-screen lg:h-screen lg:overflow-hidden">
      {/* Left panel - Order summary */}
      <div className="flex flex-col bg-background-gray gap-5 w-full lg:w-3/5 px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 xl:pb-20 border-b lg:border-b-0 lg:border-r border-border-regular lg:h-screen lg:overflow-auto">
        <div className="w-full max-w-[500px] mx-auto">
          <SubscriptionSummary
            checkoutData={checkoutData}
            couponCode={couponCode}
            onCouponCodeChange={setCouponCode}
            onApplyCoupon={handleApplyCoupon}
            onClearCoupon={handleClearCoupon}
            couponError={couponError}
            isApplyingCoupon={isApplyingCoupon}
            isPlanChange={checkoutType === CheckoutType.PLAN_CHANGE}
            isAddonPurchase={checkoutType === CheckoutType.ADDON_PURCHASE}
            planChangeContext={planChangeContext}
            addonPurchaseContext={addonPurchaseContext}
          />
        </div>
      </div>
      {/* Right panel - Payment form */}
      <CheckoutThemePane
        config={themeConfig}
        className="w-full lg:w-2/5 flex flex-col px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 lg:h-screen lg:overflow-auto shadow-md"
      >
        <div className="w-full max-w-[440px] mx-auto lg:mx-0">
          {/* Billing information */}
          <BillingInfo
            customer={customer}
            isEditing={isAddressEditing}
            setIsEditing={setIsAddressEditing}
            required={requireBillingInformation}
            onUpdated={updatedCustomer =>
              setCheckoutData(prev => ({ ...prev, customer: updatedCustomer }))
            }
          />

          {billingBlocksPayment ? (
            <div className="mt-6 p-3 bg-amber-50 text-amber-800 rounded-lg text-sm flex items-start">
              <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
              <span>
                Please provide your billing email and complete billing address above to continue.
              </span>
            </div>
          ) : (
            <>
              {/* Render based on payment availability */}
              {paymentAvailability.type === 'readonly' && (
                <ReadonlyPaymentView reason={paymentAvailability.reason} />
              )}

              {paymentAvailability.type === 'bank_only' && (
                <BankTransferInfo
                  bankAccount={paymentAvailability.bankAccount}
                  invoiceNumber={subscription?.subscription?.planName}
                  customerName={customer?.name}
                />
              )}

              {paymentAvailability.type === 'payment_form' && (
                <>
                  {hostedReturnPhase ? (
                    <ReadonlyPaymentView
                      reason="pending_payment"
                      title="Setting up your subscription"
                      message={
                        hostedReturnPhase === 'processing'
                          ? 'This will only take a moment — the page will update automatically.'
                          : "This is taking a little longer than usual. You'll get a confirmation email shortly — no need to pay again."
                      }
                    />
                  ) : (
                    <>
                      {hostedError && (
                        <div
                          className="mb-4 p-3 rounded-lg text-sm flex items-start"
                          style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
                        >
                          <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
                          <span>{hostedError}</span>
                        </div>
                      )}

                      {/* Show payment panel if card or DD available */}
                      {(paymentAvailability.cardConnectionId ||
                        paymentAvailability.directDebitConnectionId) && (
                        <PaymentPanel
                          customer={customer}
                          paymentMethods={paymentMethods || []}
                          currency={subscription.subscription.currency}
                          totalAmount={formatCurrency(
                            amountDue,
                            subscription.subscription.currency
                          )}
                          onPaymentSubmit={handlePaymentSubmit}
                          onHostedCheckout={handleHostedCheckout}
                          onPaymentMethodAttached={refreshCheckout}
                          cardConnectionId={paymentAvailability.cardConnectionId}
                          directDebitConnectionId={paymentAvailability.directDebitConnectionId}
                          themeConfig={themeConfig}
                        />
                      )}
                    </>
                  )}

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
                          invoiceNumber={subscription?.subscription?.planName}
                          customerName={customer?.name}
                        />
                      </div>
                    )}
                </>
              )}
            </>
          )}
        </div>
      </CheckoutThemePane>
    </div>
  )
}

export default CheckoutFlow
