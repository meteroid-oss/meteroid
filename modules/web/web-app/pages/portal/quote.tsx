import { Code, ConnectError } from '@connectrpc/connect'
import { Skeleton } from '@md/ui'
import { AlertCircle } from 'lucide-react'
import { ReactNode, useMemo } from 'react'

import { LinkExpired, isPortalLinkExpired } from '@/features/portal-links/LinkExpired'
import QuotePortalView from '@/features/quotes/QuotePortalView'
import { quoteBranding } from '@/features/quotes/portal/branding'
import { useQuery } from '@/lib/connectrpc'
import { getQuotePortal } from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'
import { useForceTheme } from 'providers/ThemeProvider'

import { PortalThemeProvider } from './experience/PortalThemeProvider'
import { resolveTheme } from './experience/theme'

export const PortalQuote = () => {
  const quoteQuery = useQuery(getQuotePortal)

  const data = quoteQuery.data?.quote
  const branding = useMemo(() => quoteBranding(data), [data])
  useForceTheme(resolveTheme(branding).theme)

  return (
    <PortalThemeProvider className="mtp" branding={branding}>
      <PortalQuoteContent
        isLoading={quoteQuery.isLoading}
        error={quoteQuery.error}
        content={data && <QuotePortalView key={data.quote?.documentHash} quoteData={data} />}
      />
    </PortalThemeProvider>
  )
}

const PortalQuoteContent = ({
  isLoading,
  error,
  content,
}: {
  isLoading: boolean
  error: unknown
  content: ReactNode
}) => {
  if (isPortalLinkExpired(error)) {
    return (
      <div className="flex min-h-screen w-full items-center justify-center px-4">
        <LinkExpired document="quote" />
      </div>
    )
  }

  if (
    error instanceof ConnectError &&
    error.code === Code.FailedPrecondition &&
    error.rawMessage.includes('being revised')
  ) {
    return (
      <ErrorState
        title="This quote is being revised"
        description="You will receive the new version."
      />
    )
  }

  if (error) {
    return (
      <ErrorState
        title="Quote not found"
        description="This quote link is invalid, expired, or our system is temporarily unavailable"
      />
    )
  }

  if (isLoading || !content) {
    return (
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 px-4 py-8 sm:px-6">
        <Skeleton height={16} width={50} />
        <Skeleton height={44} />
      </div>
    )
  }

  return <>{content}</>
}

const ErrorState = ({ title, description }: { title: string; description: string }) => (
  <div className="flex min-h-screen w-full items-center justify-center px-6">
    <div className="max-w-md text-center">
      <AlertCircle className="mx-auto mb-4 h-8 w-8 text-muted-foreground" />
      <h2 className="mb-2 text-md font-semibold text-foreground">{title}</h2>
      <p className="text-sm text-muted-foreground">{description}</p>
    </div>
  </div>
)
