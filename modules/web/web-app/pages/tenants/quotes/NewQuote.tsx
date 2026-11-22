import { Skeleton } from '@md/ui'
import { useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router'
import { toast } from 'sonner'

import { decodeSnapshot } from '@/features/quotes/editor/draftPayload'
import {
  localDraftKey,
  readLocalDraft,
  useLocalDraftScope,
} from '@/features/quotes/editor/localDraft'
import { quoteToTemplateSeed } from '@/features/quotes/quoteFormSeed'
import { useQuoteFormSource } from '@/features/quotes/useQuoteFormSource'
import { useQuery } from '@/lib/connectrpc'
import { getQuoteDraft } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

import { CreateQuote } from './CreateQuote'

const Loading = () => (
  <div className="space-y-4">
    <Skeleton height={32} width={200} />
    <Skeleton height={400} />
  </div>
)

/** Resumes the autosaved draft `?draft=<id>`, else falls back to the copy kept on this device. */
const ResumedQuote = ({ draftId }: { draftId: string }) => {
  const scope = useLocalDraftScope()
  const draftQuery = useQuery(
    getQuoteDraft,
    { target: { case: 'id', value: draftId } },
    { staleTime: 0, refetchOnWindowFocus: false }
  )
  const serverDraft = draftQuery.data?.draft
  const serverSeed = useMemo(
    () => (serverDraft ? decodeSnapshot(serverDraft.payload) : undefined),
    [serverDraft]
  )
  const localSeed = useMemo(() => {
    const local = readLocalDraft(localDraftKey(scope, { draftId }))
    return local ? decodeSnapshot(local.payload) : undefined
  }, [scope, draftId])

  const missing = !draftQuery.isPending && !serverSeed && !localSeed
  const loadFailed = draftQuery.isError
  useEffect(() => {
    if (missing)
      toast.error(loadFailed ? 'The draft could not be loaded' : 'This draft no longer exists')
  }, [missing, loadFailed])

  if (draftQuery.isPending) return <Loading />
  if (serverDraft && serverSeed) {
    return <CreateQuote key={draftId} seed={serverSeed} initialDraft={serverDraft} />
  }
  return <CreateQuote key={draftId} seed={localSeed} />
}

/** New quote, optionally pre-filled from an existing quote passed as `?from=<quoteId>`. */
export const NewQuote = () => {
  const [searchParams] = useSearchParams()
  // The editor writes `?draft=` itself once autosaved: only the one it was opened with is resumed.
  const [draftId] = useState(() => searchParams.get('draft') ?? undefined)
  const sourceQuoteId = draftId ? undefined : (searchParams.get('from') ?? undefined)
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

  if (draftId) return <ResumedQuote draftId={draftId} />

  if (!sourceQuoteId || isError) return <CreateQuote />

  if (!seed) return <Loading />

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
