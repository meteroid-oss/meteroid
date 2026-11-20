import { Button, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { useCallback, useState } from 'react'
import { Link } from 'react-router-dom'

import { Slot, hasSlot, placeAfter, slotItems, useVisibleItems } from '@/edition'
import { QuoteLineComments } from '@/edition/types'
import { QuoteView } from '@/features/quotes/QuoteView'
import { QuoteActivityCard } from '@/features/quotes/detail/QuoteActivityCard'
import { QuoteDetailHeader } from '@/features/quotes/detail/QuoteDetailHeader'
import { QuoteEvidence } from '@/features/quotes/detail/QuoteEvidence'
import { QuoteHistory, useUserLabel } from '@/features/quotes/detail/QuoteHistory'
import { QuoteInternalDetails } from '@/features/quotes/detail/QuoteInternalDetails'
import { QuoteSettingsTab } from '@/features/quotes/detail/QuoteSettingsTab'
import { QuoteSigners } from '@/features/quotes/detail/QuoteSigners'
import { pendingCountersignerFor } from '@/features/quotes/detail/signers'
import { QuoteActionDialogs, useQuoteActions } from '@/features/quotes/detail/useQuoteActions'
import { Band } from '@/features/subscriptions/SubscriptionAttention'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { DetailedQuote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { getQuoteApproval } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

const useNoLineComments: QuoteLineComments['useLineComments'] = () => ({
  countByLine: {},
  canComment: false,
})

interface Props {
  quote: DetailedQuote
  onChanged: () => void
}

export const QuoteDetailView = ({ quote, onChanged }: Props) => {
  const q = quote.quote
  const quoteId = q?.id ?? ''
  const currentUserEmail = useQuery(me).data?.user?.email
  const basePath = useBasePath()
  const nameOf = useUserLabel()
  const actions = useQuoteActions(quote)

  // The term shown is the quote's own minimum when set, else the plan's: the one conversion applies.
  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: q?.planVersionId ?? '' },
    { enabled: Boolean(q?.planVersionId) }
  )
  const effectiveMinimum: MinimumCommitment | undefined =
    quote.minimumCommitment ?? planQuery.data?.plan?.version?.minimumCommitment
  const planName = planQuery.data?.plan?.plan?.name

  const approvalQuery = useQuery(getQuoteApproval, { quoteId }, { enabled: Boolean(quoteId) })

  const countersigner = pendingCountersignerFor(quote, currentUserEmail)
  const canCountersign = q?.status === QuoteStatus.PENDING && Boolean(countersigner)

  const [tab, setTab] = useState('document')
  const [anchor, setAnchor] = useState<string | undefined>()
  const [focusRequest, setFocusRequest] = useState(0)
  const reply = useCallback((line?: string) => {
    setAnchor(line)
    setFocusRequest(n => n + 1)
  }, [])

  const useLineComments =
    slotItems('quoteDetail.lineComments')[0]?.useLineComments ?? useNoLineComments
  const lineComments = useLineComments(quote)
  const hasRail = hasSlot('quoteDetail.rail')

  const moduleTabs = useVisibleItems(slotItems('quoteDetail.tabs'), { quote, onChanged })
  const tabList = placeAfter<{ id: string; label: string; after?: string }>(
    [
      { id: 'document', label: 'Document' },
      { id: 'revisions', label: 'Revisions' },
      { id: 'audit', label: 'Audit trail' },
      { id: 'settings', label: 'Settings' },
    ],
    moduleTabs,
    entry => entry.id
  )

  if (!q) return null

  return (
    <div className="flex min-h-full flex-col">
      <QuoteDetailHeader
        quote={quote}
        planName={planName}
        approval={approvalQuery.data}
        approvalLoading={approvalQuery.isLoading}
        canCountersign={canCountersign}
        actions={actions}
        nameOf={nameOf}
        onChanged={onChanged}
      />

      <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-6 p-4 sm:p-6 lg:flex-row lg:items-start">
        <div className="flex min-w-0 flex-1 flex-col gap-4">
          {quote.signatureCodesUnavailable &&
            (q.status === QuoteStatus.DRAFT || q.status === QuoteStatus.PENDING) && (
              <Band
                variant="warning"
                title="Signers can't receive codes"
                actions={
                  <Button size="sm" variant="outline" asChild>
                    <Link to={`${basePath}/settings/emails`}>Email settings</Link>
                  </Button>
                }
              />
            )}
          <Slot name="quoteDetail.notices" quote={quote} onChanged={onChanged} onReply={reply} />
          {q.status === QuoteStatus.DECLINED && q.declineReason && (
            <Band
              variant="destructive"
              title="Declined"
              meta={q.declinedAt ? `on ${parseAndFormatDate(q.declinedAt)}` : undefined}
            >
              <span className="block whitespace-pre-line">{q.declineReason}</span>
            </Band>
          )}

          <Tabs value={tab} onValueChange={setTab}>
            <TabsList variant="ghost" className="mb-3" aria-label="Quote views">
              {tabList.map(entry => (
                <TabsTrigger key={entry.id} value={entry.id}>
                  {entry.label}
                </TabsTrigger>
              ))}
            </TabsList>
            <TabsContent value="document">
              <QuoteView
                quote={quote}
                mode="detailed"
                minimumCommitment={effectiveMinimum}
                totals={quote.totals}
                commentCountByLine={lineComments.countByLine}
                onLineSelect={hasRail && lineComments.canComment ? line => reply(line) : undefined}
                selectedLine={anchor}
              />
            </TabsContent>
            <TabsContent value="revisions">
              <QuoteHistory quoteId={q.id} />
            </TabsContent>
            <TabsContent value="audit">
              <QuoteEvidence quoteId={q.id} />
            </TabsContent>
            <TabsContent value="settings">
              <QuoteSettingsTab
                quote={quote}
                minimumCommitment={effectiveMinimum}
                planName={planName}
              />
            </TabsContent>
            {moduleTabs.map(({ id, Content }) => (
              <TabsContent key={id} value={id}>
                <Content quote={quote} onChanged={onChanged} />
              </TabsContent>
            ))}
          </Tabs>
        </div>

        <aside
          aria-label="Signers, discussion and activity"
          className="flex w-full flex-col gap-4 lg:w-[380px] lg:shrink-0"
        >
          <QuoteSigners
            quote={quote}
            currentUserEmail={currentUserEmail}
            onCountersign={canCountersign ? () => actions.setDialog('countersign') : undefined}
            onCopySigningLink={email => void actions.copySigningLink(email)}
            onRemind={person => actions.openSend({ name: person.name, email: person.email })}
            onChanged={onChanged}
          />
          <Slot
            name="quoteDetail.rail"
            quote={quote}
            onChanged={onChanged}
            anchor={anchor}
            onAnchorChange={setAnchor}
            focusRequest={focusRequest}
          />
          <QuoteActivityCard quoteId={q.id} />
          <QuoteInternalDetails quote={quote} nameOf={nameOf} />
        </aside>
      </div>

      <QuoteActionDialogs quote={quote} actions={actions} countersigner={countersigner} />
    </div>
  )
}
