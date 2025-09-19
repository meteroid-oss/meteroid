import { spaces } from '@md/foundation'
import { Skeleton } from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { AlertCircle } from 'lucide-react'

import QuotePortalView from '@/features/quotes/QuotePortalView'
import { useQuery } from '@/lib/connectrpc'
import { getQuotePortal } from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'

export const PortalQuote = () => {
  // useForceTheme('light')

  const quoteQuery = useQuery(getQuotePortal)

  const data = quoteQuery.data?.quote
  const error = quoteQuery.error
  const isLoading = quoteQuery.isLoading

  if (error) {
    return (
      <div className="h-full w-full ">
        <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto px-6 py-12 text-center">
          <AlertCircle className="h-8 w-8 text-muted-foreground mb-4" />
          <h2 className="text-md font-semibold text-gray-800 mb-2">Quote not found</h2>
          <p className="text-gray-800 text-sm">
            This quote link is invalid, expired, or our system is temporarily unavailable
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full ">
      <Flex direction="column" gap={spaces.space6} fullHeight>
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <QuotePortalView quoteData={data} />
        )}
      </Flex>
    </div>
  )
}
