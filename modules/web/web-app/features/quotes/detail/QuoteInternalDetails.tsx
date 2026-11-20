import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@md/ui'
import { ChevronDown, ExternalLink } from 'lucide-react'

import { DetailedQuote } from '@/rpc/api/quotes/v1/models_pb'

const isUrl = (value: string) => /^https?:\/\//i.test(value)

/** What only the team sees: the owner, the CRM deal, the internal notes. */
export const QuoteInternalDetails = ({
  quote,
  nameOf,
}: {
  quote: DetailedQuote
  nameOf: (userId?: string) => string
}) => {
  const q = quote.quote
  if (!q) return null
  const deal = q.hubspotDealId?.trim()
  const notes = q.internalNotes?.trim()
  const parts = [q.ownerId && 'owner', deal && 'HubSpot', notes && 'notes'].filter(Boolean)

  return (
    <Collapsible className="rounded-lg border border-border bg-card">
      <CollapsibleTrigger className="group flex w-full items-center justify-between gap-2 px-4 py-3 text-left">
        <span className="text-sm font-semibold text-foreground">
          Deal & internal
          {parts.length > 0 && (
            <span className="font-normal text-muted-foreground"> · {parts.join(', ')}</span>
          )}
        </span>
        <ChevronDown
          aria-hidden
          className="h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-3 border-t border-border px-4 py-3 text-[13px]">
        {parts.length === 0 && <p className="text-muted-foreground">Nothing recorded</p>}
        {q.ownerId && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Owner</span>
            <span className="text-foreground">{nameOf(q.ownerId)}</span>
          </div>
        )}
        {deal && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">HubSpot deal</span>
            {isUrl(deal) ? (
              <a
                href={deal}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex min-w-0 items-center gap-1 text-brand hover:underline"
              >
                <span className="truncate">Open the deal</span>
                <ExternalLink className="h-3 w-3 shrink-0" aria-hidden />
              </a>
            ) : (
              <span className="font-mono text-foreground">{deal}</span>
            )}
          </div>
        )}
        {notes && (
          <div className="flex flex-col gap-1">
            <span className="font-medium text-warning">Internal notes</span>
            <p className="whitespace-pre-line text-muted-foreground">{notes}</p>
          </div>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}
