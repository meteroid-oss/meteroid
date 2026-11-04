import { Button, Skeleton } from '@md/ui'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { isQuoteEditable } from '@/features/quotes/quoteEditable'
import { quoteToFormSeed } from '@/features/quotes/quoteFormSeed'
import { useQuoteFormSource } from '@/features/quotes/useQuoteFormSource'
import { useBasePath } from '@/hooks/useBasePath'
import { useTypedParams } from '@/utils/params'

import { CreateQuote } from './CreateQuote'

export const EditQuote = () => {
  const { quoteId } = useTypedParams<{ quoteId: string }>()
  const basePath = useBasePath()
  const { quote, planComponents } = useQuoteFormSource(quoteId)

  const seed = useMemo(
    () => (quote && planComponents ? quoteToFormSeed(quote, planComponents) : undefined),
    [quote, planComponents]
  )

  if (quote && !isQuoteEditable(quote)) {
    return (
      <div className="space-y-4">
        <p className="text-muted-foreground">
          Only drafts and sent quotes nobody has signed yet can be edited.
        </p>
        <Button variant="outline" asChild>
          <Link to={`${basePath}/quotes/${quoteId}`}>Back to quote</Link>
        </Button>
      </div>
    )
  }

  if (!seed) {
    return (
      <div className="space-y-4">
        <Skeleton height={32} width={200} />
        <Skeleton height={400} />
      </div>
    )
  }

  return <CreateQuote key={quoteId} seed={seed} editQuoteId={quoteId} />
}
