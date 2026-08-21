import { Code, ConnectError } from '@connectrpc/connect'
import { Skeleton } from '@md/ui'
import { AlertCircle } from 'lucide-react'
import { useEffect } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'

import CheckoutFlow from '@/features/checkout/CheckoutFlow'
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
  const error = checkoutQuery.error
  const isLoading = checkoutQuery.isLoading

  // A completed session is a SUCCESS, not an error: the webhook can finish the
  // checkout before (or during) the customer's redirect back — common for the
  // direct-debit flow, where activation is webhook-driven — in which case this
  // first GetCheckout races and fails. Send them to the success page instead of
  // the generic error. A `gocardless_status=ok` return means the direct-debit
  // payment is submitted (settling later), so mark it "processing".
  const sessionCompleted =
    error instanceof ConnectError &&
    error.code === Code.FailedPrecondition &&
    error.message.toLowerCase().includes('already been completed')

  useEffect(() => {
    if (!sessionCompleted) return
    const params = new URLSearchParams()
    const returnUrl = searchParams.get('return_url')
    if (returnUrl) params.set('return_url', returnUrl)
    if (searchParams.get('gocardless_status') === 'ok') params.set('status', 'processing')
    navigate(`success?${params.toString()}`, { replace: true })
  }, [sessionCompleted, navigate, searchParams])

  if (sessionCompleted) {
    // Redirecting to success; avoid flashing the error panel.
    return null
  }

  if (error) {
    return (
      <div className="h-full w-full bg-[#00000002]">
        <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto px-6 py-12 text-center">
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
          <CheckoutFlow
            checkoutData={data}
            checkoutType={checkoutType}
            planChangeContext={planChangeContext}
            addonPurchaseContext={addonPurchaseContext}
          />
        )}
      </div>
    </div>
  )
}
