import { CheckCircle, Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useLocation, useSearchParams } from 'react-router-dom'

import { CheckoutSuccessState } from '@/features/checkout/utils/checkoutRedirect'
import { useForceTheme } from 'providers/ThemeProvider'

export const PortalCheckoutSuccess = () => {
  useForceTheme('light')
  const [searchParams] = useSearchParams()
  // Only router state can carry the redirect target: a URL parameter would make this
  // unauthenticated page an open redirect on the merchant's domain.
  const state = useLocation().state as CheckoutSuccessState | null
  const returnUrl = state?.returnUrl
  const backTo = state?.backTo
  // Direct debit is submitted at checkout but settles days later, so the payment
  // isn't "successful" yet — the subscription is active and the payment is
  // processing. Card checkouts settle inline and stay on the default wording.
  const processing = searchParams.get('status') === 'processing'
  // Set by the invoice-payment flow; absent on checkout.
  const invoiceNumber = searchParams.get('invoice')
  const topUp = searchParams.get('purchase') === 'top-up'
  const [countdown, setCountdown] = useState(3)

  useEffect(() => {
    if (!returnUrl) return

    const timer = setInterval(() => {
      setCountdown(prev => {
        if (prev <= 1) {
          clearInterval(timer)
          window.location.href = returnUrl
          return 0
        }
        return prev - 1
      })
    }, 1000)

    return () => clearInterval(timer)
  }, [returnUrl])

  return (
    <div className="min-h-screen w-full bg-background flex flex-col items-center justify-center px-6 py-12 text-center">
      <div className="w-full max-w-md flex flex-col items-center">
        <CheckCircle className="h-12 w-12 text-success mb-4 " />
        <h2 className="text-md font-semibold text-foreground mb-2">
          {processing
            ? invoiceNumber
              ? 'Payment submitted'
              : "You're all set!"
            : 'Payment successful!'}
        </h2>
        {returnUrl ? (
          <div className="text-foreground text-sm">
            <div className="flex items-center justify-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>Redirecting in {countdown}...</span>
            </div>
            <a href={returnUrl} className="text-brand hover:underline mt-2 block">
              Click here if not redirected
            </a>
          </div>
        ) : backTo ? (
          <div className="text-foreground text-sm">
            <p>
              {processing && invoiceNumber
                ? `Your payment for invoice ${invoiceNumber} is being processed and will be collected shortly.`
                : 'Thank you for your payment.'}
            </p>
            <a href={backTo.url} className="text-brand hover:underline mt-2 block">
              {backTo.label}
            </a>
          </div>
        ) : (
          <p className="text-foreground text-sm">
            {processing
              ? topUp
                ? 'Your payment is processing. Credit will be added to your balance once it settles. You can safely close this tab.'
                : invoiceNumber
                  ? `Your payment for invoice ${invoiceNumber} is being processed and will be collected shortly. You can safely close this tab.`
                  : 'Your subscription is confirmed. Your first payment is being processed and will be collected shortly. You can safely close this tab.'
              : 'Thank you for your payment. You can now safely close this tab.'}
          </p>
        )}
      </div>
    </div>
  )
}
