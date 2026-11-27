import { Button, Skeleton } from '@md/ui'
import { TriangleAlert } from 'lucide-react'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { DetailPageFrame } from '@/components/layouts/DetailLayout'
import { QuoteDetailView } from '@/features/quotes/detail/QuoteDetailView'
import { useInvalidateQuote } from '@/features/quotes/detail/useQuoteActions'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { getQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Quote = () => {
  const { quoteId } = useTypedParams<{ quoteId: string }>()
  const quoteQuery = useQuery(getQuote, { id: quoteId ?? '' }, { enabled: Boolean(quoteId) })
  const invalidate = useInvalidateQuote()
  const data = quoteQuery.data?.quote
  const basePath = useBasePath()

  // While it loads, or when it can't, the page keeps its frame so nothing jumps.
  const section = { label: 'Quotes', to: `${basePath}/quotes` }

  if (quoteQuery.isError && !data) {
    return (
      <DetailPageFrame section={section} title="Quote">
        <div className="p-6">
          <SectionEmptyState
            className="bg-card"
            icon={<TriangleAlert />}
            title="This quote could not be loaded"
            action={
              <Button size="sm" variant="secondary" onClick={() => void quoteQuery.refetch()}>
                Try again
              </Button>
            }
          >
            {quoteQuery.error.message}
          </SectionEmptyState>
        </div>
      </DetailPageFrame>
    )
  }

  if (quoteQuery.isLoading || !data) {
    return (
      <DetailPageFrame section={section} title="Quote">
        <div className="flex flex-col gap-3 p-6">
          <Skeleton height={16} width={120} />
          <Skeleton height={28} width={360} />
          <Skeleton height={20} width={520} />
          <div className="mt-4 flex flex-col gap-6 lg:flex-row">
            <Skeleton height={480} className="flex-1" />
            <Skeleton height={320} className="lg:w-[380px]" />
          </div>
        </div>
      </DetailPageFrame>
    )
  }

  return <QuoteDetailView quote={data} onChanged={() => void invalidate()} />
}
