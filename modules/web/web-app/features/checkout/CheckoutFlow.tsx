import { Code, ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { AlertCircle, ArrowLeft } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'

import { CheckoutThemePane } from '@/features/checkout/CheckoutThemePane'
import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { BillingInfo } from '@/features/checkout/components/BillingInfo'
import { ReadonlyPaymentView } from '@/features/checkout/components/ReadonlyPaymentView'
import { resolveCheckoutTheme } from '@/features/checkout/resolveCheckoutTheme'
import { hasCompleteBillingInformation } from '@/features/checkout/utils/billingInfo'
import {
  CHECKOUT_SUCCESS_PATH,
  CheckoutSuccessState,
  legacyReturnUrl,
  stashCheckoutRedirect,
} from '@/features/checkout/utils/checkoutRedirect'
import { completeNextAction } from '@/features/checkout/utils/completeNextAction'
import {
  HostedRail,
  consumeHostedReturn,
  hostedReturnErrorMessage,
  hostedReturnUrl,
} from '@/features/checkout/utils/hostedReturn'
import { getCheckoutPaymentAvailability } from '@/features/checkout/utils/paymentAvailability'
import { checkoutSubmitLabel } from '@/features/checkout/utils/submitLabel'
import { BankTransferInfo } from '@/features/invoice-payment/components/BankTransferInfo'
import { topUpSettlesCopy } from '@/pages/portal/experience/screens/creditCopy'
import { walletAmount } from '@/pages/portal/experience/walletFormat'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  confirmCheckout,
  getCheckout,
  initiateHostedCheckout,
} from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { CheckoutType, ConfirmCheckoutStatus } from '@/rpc/portal/checkout/v1/checkout_pb'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'
import { ConnectionTypeEnum } from '@/rpc/portal/shared/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

import { SubscriptionSummary } from './components/SubscriptionSummary'
import { CheckoutFlowProps } from './types'

// After a hosted flow returns `ok`, the backend creates the subscription (via webhook, which can
// lag the redirect, or via the return handler), so poll until the session is completed.
const HOSTED_ACTIVATION_POLL_MS = 3000
const HOSTED_ACTIVATION_TIMEOUT_MS = 2 * 60 * 1000

const CheckoutFlow: React.FC<CheckoutFlowProps> = ({
  checkoutData: initialCheckoutData,
  checkoutType,
  planChangeContext,
  addonPurchaseContext,
  successUrl: sessionSuccessUrl,
  cancelUrl,
}) => {
  const [isAddressEditing, setIsAddressEditing] = useState(
    initialCheckoutData.requireBillingInformation &&
      !hasCompleteBillingInformation(initialCheckoutData.customer)
  )
  const [couponCode, setCouponCode] = useState('')
  const [couponError, setCouponError] = useState<string | undefined>(undefined)
  // The code the backend last accepted; cleared as soon as the customer edits the input.
  const [validCouponCode, setValidCouponCode] = useState<string | undefined>(undefined)
  const [isApplyingCoupon, setIsApplyingCoupon] = useState(false)
  const [checkoutData, setCheckoutData] = useState<Checkout>(initialCheckoutData)
  // Hosted-flow round trip (GoCardless mandate / Stancer card): the customer is
  // redirected back here with a hosted_status param.
  // Lazy initializer so the params are read (and stripped) exactly once — a
  // re-run of the mount effect (StrictMode) must see the same outcome.
  const [hostedReturn] = useState(() => consumeHostedReturn())
  const [hostedError, setHostedError] = useState<string | null>(null)
  // On an `ok` or `processing` hosted-checkout return the first payment is submitted and the
  // backend finalizes everything. The frontend only observes: 'processing' while polling,
  // 'delayed' past timeout.
  const [hostedReturnPhase, setHostedReturnPhase] = useState<'processing' | 'delayed' | null>(() =>
    hostedReturn?.status === 'ok' || hostedReturn?.status === 'processing' ? 'processing' : null
  )
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const successUrl = sessionSuccessUrl ?? legacyReturnUrl(searchParams)
  useEffect(() => {
    stashCheckoutRedirect({ successUrl, cancelUrl })
  }, [successUrl, cancelUrl])
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
    directDebitCapabilities,
    bankAccount,
    topUpCredits,
    requireBillingInformation,
  } = checkoutData

  // A hosted payment already under way was started with complete billing information.
  const billingBlocksPayment =
    !hostedReturnPhase && requireBillingInformation && !hasCompleteBillingInformation(customer)

  const currency = checkoutData.topUpCurrency ?? subscription?.subscription?.currency
  const isTopUp = checkoutType === CheckoutType.TOP_UP

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
      if (isTopUp) params.set('purchase', 'top-up')
      if (processing) {
        params.set('status', 'processing')
      }
      const state: CheckoutSuccessState = { returnUrl: successUrl }
      navigate(`${CHECKOUT_SUCCESS_PATH}?${params.toString()}`, { state })
    },
    [subscription, customer, successUrl, navigate, isTopUp]
  )

  // Re-pull the checkout (preserving any applied coupon) so a freshly attached
  // payment method appears in the list.
  const refreshCheckout = useCallback(async () => {
    try {
      // Only a code the backend accepted: a rejected one would make this refresh fail too.
      const response = await applyCouponMutation.mutateAsync(
        validCouponCode ? { couponCode: validCouponCode } : {}
      )
      if (response.checkout) {
        setCheckoutData(response.checkout)
      }
    } catch {
      // best-effort refresh; keep current data on failure
    }
  }, [applyCouponMutation, validCouponCode])

  const handleApplyCoupon = async () => {
    const code = couponCode.trim()
    if (!code) return

    setIsApplyingCoupon(true)
    setCouponError(undefined)
    setValidCouponCode(undefined)

    try {
      const response = await applyCouponMutation.mutateAsync({
        couponCode: code,
      })

      if (response.checkout) {
        setCheckoutData(response.checkout)
      }
      setValidCouponCode(code)
    } catch (error) {
      if (
        error instanceof ConnectError &&
        (error.code === Code.NotFound || error.code === Code.InvalidArgument)
      ) {
        setCouponError(error.rawMessage || 'This code is not valid.')
      } else {
        setCouponError('Failed to apply coupon')
      }
    } finally {
      setIsApplyingCoupon(false)
    }
  }

  const handleCouponCodeChange = (code: string) => {
    setCouponCode(code)
    setCouponError(undefined)
    setValidCouponCode(undefined)
  }

  const handleClearCoupon = async () => {
    setCouponCode('')
    setCouponError(undefined)
    setValidCouponCode(undefined)

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

      if (!currency) {
        throw new Error('Currency is not defined')
      }

      const res = await confirmCheckoutMutation.mutateAsync({
        displayedAmount: amountDue,
        displayedCurrency: currency,
        paymentMethodId,
        couponCode: couponCode.trim() || undefined,
      })

      // If the charge needs 3DS/SCA, complete it before navigating.
      await completeNextAction(res.nextAction)

      // Not settled yet (direct debit, a charge the provider confirms later): "processing".
      navigateToSuccess(res.status === ConfirmCheckoutStatus.AWAITING_PAYMENT)
    } catch (error) {
      console.error('Payment submission error:', error)

      if (
        error instanceof ConnectError &&
        /coupon|code/i.test(error.rawMessage) &&
        (error.code === Code.NotFound || error.code === Code.InvalidArgument)
      ) {
        // Surface the coupon issue AND keep the panel retryable — resolving
        // here would flip the panel to a false SUCCESS while nothing was
        // charged. Throwing keeps it in ERROR; a retry reuses the already
        // attached method (PaymentPanel remembers it) rather than re-confirming
        // the consumed SetupIntent.
        const message = error.rawMessage
        setCouponError(message)
        throw new Error(message)
      }

      throw error // Let the PaymentPanel handle this error
    }
  }

  // Hosted checkout: ONE explicit customer action. The RPC validates the
  // displayed amount server-side and returns a redirect next_action we follow.
  const handleHostedCheckout = async (connectionId: string, rail: HostedRail) => {
    setCouponError(undefined)

    if (billingBlocksPayment) {
      setIsAddressEditing(true)
      throw new Error('Please complete your billing information before continuing.')
    }

    if (!currency) {
      throw new Error('Currency is not defined')
    }

    const res = await initiateHostedCheckoutMutation.mutateAsync({
      connectionId,
      displayedAmount: amountDue,
      displayedCurrency: currency,
      couponCode: couponCode.trim() || undefined,
      returnUrl: hostedReturnUrl(),
      // Only matters when one connection serves both rails (Mollie).
      connectionType: rail === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
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

    // `processing` is still settling: treat it like `ok` rather than prompt a second payment.
    if (ret.status !== 'ok' && ret.status !== 'processing') {
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

  if ((!subscription?.subscription && !isTopUp) || !customer || !currency) {
    return <div className="p-8 text-center">Loading checkout information...</div>
  }

  const topUpSettles = topUpSettlesCopy(topUpCredits?.creditUnit)
  const paymentAvailability = getCheckoutPaymentAvailability({
    subscriptionStatus: subscription?.subscription?.status,
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
          {cancelUrl && (
            <a
              href={cancelUrl}
              className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:underline mb-4"
            >
              <ArrowLeft className="h-4 w-4" />
              Back to {checkoutData.tradeName}
            </a>
          )}
          {isTopUp ? (
            <div className="space-y-4">
              <h1 className="text-xl font-semibold">
                {topUpCredits?.creditUnit
                  ? `Buy ${topUpCredits.creditUnit.pluralName}`
                  : 'Add credit'}
              </h1>
              <p className="text-muted-foreground">
                {topUpSettles.charAt(0).toUpperCase() + topUpSettles.slice(1)}
              </p>
              {topUpCredits?.creditUnit && (
                <p className="text-lg font-medium">
                  {walletAmount(topUpCredits.amount, currency, topUpCredits.creditUnit)}
                </p>
              )}
              <p className="text-2xl font-semibold">{formatCurrency(amountDue, currency)}</p>
              {checkoutData.taxAmount > BigInt(0) && (
                <div className="space-y-1 border-t border-border pt-4 text-sm">
                  <div className="flex justify-between">
                    <div>Subtotal</div>
                    <div>{formatCurrency(checkoutData.subtotalAmount, currency)}</div>
                  </div>
                  <div className="flex justify-between">
                    <div>Tax</div>
                    <div>{formatCurrency(checkoutData.taxAmount, currency)}</div>
                  </div>
                  <div className="flex justify-between font-medium">
                    <div>Total due today</div>
                    <div>{formatCurrency(amountDue, currency)}</div>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <SubscriptionSummary
              checkoutData={checkoutData}
              couponCode={couponCode}
              onCouponCodeChange={handleCouponCodeChange}
              isCouponValid={!!validCouponCode}
              onApplyCoupon={handleApplyCoupon}
              onClearCoupon={handleClearCoupon}
              couponError={couponError}
              isApplyingCoupon={isApplyingCoupon}
              isPlanChange={checkoutType === CheckoutType.PLAN_CHANGE}
              isAddonPurchase={checkoutType === CheckoutType.ADDON_PURCHASE}
              planChangeContext={planChangeContext}
              addonPurchaseContext={addonPurchaseContext}
            />
          )}
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
            onUpdated={updatedCustomer => {
              // Unblocks the payment at once; the refresh then re-prices from the new address.
              setCheckoutData(prev => ({ ...prev, customer: updatedCustomer }))
              refreshCheckout()
            }}
          />

          {billingBlocksPayment && (
            <div
              className="mt-6 p-3 rounded-lg text-sm flex items-start"
              style={{ background: 'var(--mtp-warn-bg)', color: 'var(--mtp-warn-text)' }}
            >
              <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
              <span>
                Please provide your billing email and complete billing address above to continue.
              </span>
            </div>
          )}

          {/* Render based on payment availability */}
          {!billingBlocksPayment && paymentAvailability.type === 'readonly' && (
            <ReadonlyPaymentView reason={paymentAvailability.reason} />
          )}

          {!billingBlocksPayment && paymentAvailability.type === 'bank_only' && (
            <BankTransferInfo
              bankAccount={paymentAvailability.bankAccount}
              invoiceNumber={subscription?.subscription?.planName}
              customerName={customer?.name}
            />
          )}

          {!billingBlocksPayment && paymentAvailability.type === 'payment_form' && (
            <>
              {hostedReturnPhase ? (
                <ReadonlyPaymentView
                  reason="pending_payment"
                  title={isTopUp ? 'Adding your credit' : 'Setting up your subscription'}
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
                      submitLabel={checkoutSubmitLabel(checkoutType)}
                      currency={currency}
                      totalAmount={formatCurrency(amountDue, currency)}
                      onPaymentSubmit={handlePaymentSubmit}
                      onHostedCheckout={handleHostedCheckout}
                      onPaymentMethodAttached={refreshCheckout}
                      cardConnectionId={paymentAvailability.cardConnectionId}
                      directDebitConnectionId={paymentAvailability.directDebitConnectionId}
                      directDebitCapabilities={directDebitCapabilities}
                      themeConfig={themeConfig}
                      showPoweredBy={initialCheckoutData.showPoweredBy}
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
        </div>
      </CheckoutThemePane>
    </div>
  )
}

export default CheckoutFlow
