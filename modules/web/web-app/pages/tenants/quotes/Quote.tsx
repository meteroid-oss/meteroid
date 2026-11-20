import { Skeleton } from '@md/ui'

import { QuoteDetailView } from '@/features/quotes/detail/QuoteDetailView'
import { useInvalidateQuote } from '@/features/quotes/detail/useQuoteActions'
import { useQuery } from '@/lib/connectrpc'
import { getQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Quote = () => {
  const { quoteId } = useTypedParams<{ quoteId: string }>()
  const quoteQuery = useQuery(getQuote, { id: quoteId ?? '' }, { enabled: Boolean(quoteId) })
  const invalidate = useInvalidateQuote()
  const data = quoteQuery.data?.quote

  if (quoteQuery.isError && !data) {
    return (
      <p className="p-6 text-sm text-muted-foreground">
        This quote could not be loaded: {quoteQuery.error.message}
      </p>
    )
  }

  if (quoteQuery.isLoading || !data) {
    return (
      <div className="flex flex-col gap-3 p-6">
        <Skeleton height={16} width={120} />
        <Skeleton height={28} width={360} />
        <Skeleton height={20} width={520} />
        <div className="mt-4 flex flex-col gap-6 lg:flex-row">
          <Skeleton height={480} className="flex-1" />
          <Skeleton height={320} className="lg:w-[380px]" />
        </div>
      </div>
    )
  }

  return <QuoteDetailView quote={data} onChanged={() => void invalidate()} />
}
