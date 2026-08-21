import { CheckCircle, Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { useForceTheme } from 'providers/ThemeProvider'

export const PortalCheckoutSuccess = () => {
  useForceTheme('light')
  const [searchParams] = useSearchParams()
  const returnUrl = searchParams.get('return_url')
  // Direct debit is submitted at checkout but settles days later, so the payment
  // isn't "successful" yet — the subscription is active and the payment is
  // processing. Card checkouts settle inline and stay on the default wording.
  const processing = searchParams.get('status') === 'processing'
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
    <div className="min-h-screen w-full bg-[#00000002] flex flex-col items-center justify-center px-6 py-12 text-center">
      <div className="w-full max-w-md flex flex-col items-center">
        <CheckCircle className="h-12 w-12 text-success mb-4 " />
        <h2 className="text-md font-semibold text-gray-800 mb-2">
          {processing ? "You're all set!" : 'Payment successful!'}
        </h2>
        {returnUrl ? (
          <div className="text-gray-800 text-sm">
            <div className="flex items-center justify-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>Redirecting in {countdown}...</span>
            </div>
            <a href={returnUrl} className="text-blue-600 hover:underline mt-2 block">
              Click here if not redirected
            </a>
          </div>
        ) : (
          <p className="text-gray-800 text-sm">
            {processing
              ? 'Your subscription is confirmed. Your first payment is being processed and will be collected shortly. You can safely close this tab.'
              : 'Thank you for your payment. You can now safely close this tab.'}
          </p>
        )}
      </div>
    </div>
  )
}
