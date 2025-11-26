import { spaces } from '@md/foundation'
import { Skeleton } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { AlertCircle } from 'lucide-react'

import CheckoutFlow from '@/features/checkout/CheckoutFlow'
import { useQuery } from '@/lib/connectrpc'
import { getSubscriptionCheckout } from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { useTypedParams } from '@/utils/params'
import { useForceTheme } from 'providers/ThemeProvider'

export const PortalCheckout = () => {
  useForceTheme('light')

  const subscriptionId = useTypedParams<{ subscriptionId: string }>().subscriptionId

  const checkoutQuery = useQuery(getSubscriptionCheckout, {
    subscriptionId,
  })

  const data = checkoutQuery.data?.checkout
  const error = checkoutQuery.error
  const isLoading = checkoutQuery.isLoading

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
      <Flex direction="column" gap={spaces.space6} fullHeight>
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <CheckoutFlow checkoutData={data} />
        )}
      </Flex>
    </div>
  )
}
