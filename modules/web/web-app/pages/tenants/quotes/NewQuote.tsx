import { Skeleton } from '@md/ui'
import { useMemo } from 'react'
import { useSearchParams } from 'react-router-dom'

import { quoteToTemplateSeed } from '@/features/quotes/quoteFormSeed'
import { useQuoteFormSource } from '@/features/quotes/useQuoteFormSource'

import { CreateQuote } from './CreateQuote'

/** New quote, optionally pre-filled from an existing quote passed as `?from=<quoteId>`. */
export const NewQuote = () => {
  const [searchParams] = useSearchParams()
  const sourceQuoteId = searchParams.get('from') ?? undefined
  const customerId = searchParams.get('customerId')?.trim() ?? ''
  const { quote, planComponents, planVersionIsActive, isReady, isError } =
    useQuoteFormSource(sourceQuoteId)

  const seed = useMemo(
    () =>
      isReady && quote && planComponents
        ? quoteToTemplateSeed(quote, planComponents, { customerId, planVersionIsActive })
        : undefined,
    [isReady, quote, planComponents, customerId, planVersionIsActive]
  )

  if (!sourceQuoteId || isError) return <CreateQuote />

  if (!seed) {
    return (
      <div className="space-y-4">
        <Skeleton height={32} width={200} />
        <Skeleton height={400} />
      </div>
    )
  }

  return (
    <CreateQuote
      key={sourceQuoteId}
      seed={seed}
      template={{
        sourceQuoteNumber: quote?.quote?.quoteNumber ?? '',
        planOutdated: !planVersionIsActive,
      }}
    />
  )
}
