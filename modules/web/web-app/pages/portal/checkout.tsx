import { Code, ConnectError } from '@connectrpc/connect'
import { AlertCircle, Loader2 } from 'lucide-react'
import { useEffect } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'

import CheckoutFlow from '@/features/checkout/CheckoutFlow'
import {
  CHECKOUT_SUCCESS_PATH,
  CheckoutSuccessState,
  consumeCheckoutRedirect,
  legacyReturnUrl,
} from '@/features/checkout/utils/checkoutRedirect'
import { HOSTED_STATUS_PARAM } from '@/features/checkout/utils/hostedReturn'
import { useQuery } from '@/lib/connectrpc'
import { getCheckout } from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { useForceTheme } from 'providers/ThemeProvider'

export const PortalCheckout = () => {
  useForceTheme('light')
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()

  // Use the unified GetCheckout endpoint - token is passed via authorization header
  const checkoutQuery = useQuery(getCheckout, {})

  const data = checkoutQuery.data?.checkout
  const checkoutType = checkoutQuery.data?.checkoutType
  const planChangeContext = checkoutQuery.data?.planChangeContext
  const addonPurchaseContext = checkoutQuery.data?.addonPurchaseContext
  const successUrl = checkoutQuery.data?.successUrl
  const cancelUrl = checkoutQuery.data?.cancelUrl
  const error = checkoutQuery.error
  const isLoading = checkoutQuery.isLoading
  // On the return leg from a hosted provider the URL carries `hosted_status`;
  // show "confirming your payment" rather than a generic loader while we resolve.
  const hostedStatus = searchParams.get(HOSTED_STATUS_PARAM)
  const isHostedReturn = hostedStatus === 'ok' || hostedStatus === 'processing'

  // A completed session is a SUCCESS, not an error: the backend can finish
  // the checkout before (or during) the redirect back, in which case this
  // first GetCheckout fails — send them to the success page. An `ok`/`processing` hosted
  // return means the payment is submitted (settling later): "processing".
  const sessionCompleted =
    error instanceof ConnectError &&
    error.code === Code.FailedPrecondition &&
    error.message.toLowerCase().includes('already been completed')

  useEffect(() => {
    if (!sessionCompleted) return
    const params = new URLSearchParams()
    if (isHostedReturn) {
      params.set('status', 'processing')
    }
    // GetCheckout no longer answers for a completed session: the redirect target was stashed
    // by CheckoutFlow before the customer left for the hosted flow.
    const state: CheckoutSuccessState = {
      returnUrl: consumeCheckoutRedirect().successUrl ?? legacyReturnUrl(searchParams),
    }
    navigate(`${CHECKOUT_SUCCESS_PATH}?${params.toString()}`, { replace: true, state })
  }, [sessionCompleted, isHostedReturn, navigate, searchParams])

  if (sessionCompleted) {
    // Redirecting to success; avoid flashing the error panel.
    return null
  }

  if (error) {
    // Refusals the customer can act on (a promo code already redeemed) arrive worded for them.
    const refusal =
      error instanceof ConnectError &&
      (error.code === Code.InvalidArgument || error.code === Code.FailedPrecondition)
        ? error.rawMessage
        : undefined
    return (
      <div className="h-full w-full bg-[#00000002]">
        <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto px-6 py-12 text-center">
          <AlertCircle className="h-8 w-8 text-muted-foreground mb-4" />
          <h2 className="text-md font-semibold text-gray-800 mb-2">
            {refusal ? "This checkout can't be completed" : 'Something went wrong'}
          </h2>
          <p className="text-gray-800 text-sm">
            {refusal ??
              'There may be a connection issue, your session might be expired or completed, or our payment system is temporarily unavailable'}
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-[#00000002]">
      <div className="flex flex-col gap-4 h-full">
        {isLoading || !data ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-center">
            <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            <p className="text-sm text-muted-foreground">
              {isHostedReturn ? 'Confirming your payment…' : 'Loading your checkout…'}
            </p>
          </div>
        ) : (
          <CheckoutFlow
            checkoutData={data}
            checkoutType={checkoutType}
            planChangeContext={planChangeContext}
            addonPurchaseContext={addonPurchaseContext}
            successUrl={successUrl}
            cancelUrl={cancelUrl}
          />
        )}
      </div>
    </div>
  )
}
