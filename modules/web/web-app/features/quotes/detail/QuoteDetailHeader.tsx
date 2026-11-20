import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import {
  ArrowLeftRight,
  CopyIcon,
  CopyPlus,
  Download,
  ExternalLink,
  Eye,
  FileText,
  MoreHorizontal,
  PenLine,
  Pencil,
  Send,
  ThumbsDown,
  XCircle,
} from 'lucide-react'
import { ReactNode } from 'react'
import { Link } from 'react-router-dom'

import { slotItems } from '@/edition'
import {
  ChangesRequestedBadge,
  QuoteApprovalBadge,
  QuoteStatusBadge,
} from '@/features/quotes/QuoteStatusBadge'
import { QuoteProgress } from '@/features/quotes/detail/QuoteProgress'
import { fromNow, recurringTotalLabel } from '@/features/quotes/detail/quoteFormat'
import { QuoteActions } from '@/features/quotes/detail/useQuoteActions'
import { isQuoteEditable } from '@/features/quotes/quoteEditable'
import { useBasePath } from '@/hooks/useBasePath'
import { parseUtc } from '@/lib/utils/date'
import { DetailedQuote, QuoteApprovalStatus, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { GetQuoteApprovalResponse } from '@/rpc/api/quotes/v1/quotes_pb'

interface Props {
  quote: DetailedQuote
  planName?: string
  approval?: GetQuoteApprovalResponse
  approvalLoading: boolean
  canCountersign: boolean
  actions: QuoteActions
  nameOf: (userId?: string) => string
  onChanged: () => void
}

/** A disabled button still explains itself: the tooltip needs an enabled wrapper. */
const Explained = ({ reason, children }: { reason: string; children: ReactNode }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span tabIndex={0}>{children}</span>
    </TooltipTrigger>
    <TooltipContent className="max-w-xs">{reason}</TooltipContent>
  </Tooltip>
)

const expiryLabel = (expiresAt: string) => {
  const past = parseUtc(expiresAt).getTime() < Date.now()
  return past ? `expired ${fromNow(expiresAt)}` : `expires ${fromNow(expiresAt)}`
}

export const QuoteDetailHeader = ({
  quote,
  planName,
  approval,
  approvalLoading,
  canCountersign,
  actions,
  nameOf,
  onChanged,
}: Props) => {
  const basePath = useBasePath()
  const q = quote.quote
  if (!q) return null

  const status = q.status
  const isDraft = status === QuoteStatus.DRAFT
  const isPending = status === QuoteStatus.PENDING
  const canEdit = isQuoteEditable(quote)
  const canSend = isDraft || isPending
  const canCancel = isDraft || isPending
  const emailEnabled = quote.quoteEmailEnabled !== false
  const needsApproval =
    isDraft && (approval?.matchedRules.length ?? 0) > 0 && approval?.satisfied !== true
  const ApprovalAction = slotItems('quoteDetail.approvalAction')[0]

  const customerName = quote.customer?.name || q.customerName || 'Customer'
  const title = planName ? `${customerName} · ${planName}` : customerName
  const recurring = quote.totals ? recurringTotalLabel(quote.totals) : undefined
  const expiry = canSend && q.expiresAt ? expiryLabel(q.expiresAt) : undefined
  const facts = [recurring, expiry].filter(Boolean).join(' · ')

  const copyLink = () => {
    if (q.recipients.length === 1) void actions.copySigningLink(q.recipients[0].email)
    else actions.setDialog('link')
  }

  const primary = (() => {
    if (isDraft) {
      if (approvalLoading) {
        return (
          <Button size="sm" hasIcon disabled>
            <Send className="h-4 w-4" />
            Send to customer
          </Button>
        )
      }
      if (needsApproval) {
        return ApprovalAction ? (
          <ApprovalAction quote={quote} onChanged={onChanged} />
        ) : (
          <Explained reason="Needs approval">
            <Button size="sm" hasIcon disabled>
              <Send className="h-4 w-4" />
              Send to customer
            </Button>
          </Explained>
        )
      }
      return emailEnabled ? (
        <Button size="sm" hasIcon onClick={() => actions.openSend()}>
          <Send className="h-4 w-4" />
          Send to customer
        </Button>
      ) : (
        <Explained reason="Quote emails are off">
          <Button
            size="sm"
            hasIcon
            disabled={actions.pending.publish}
            onClick={() => void actions.publish()}
          >
            <FileText className="h-4 w-4" />
            Publish
          </Button>
        </Explained>
      )
    }
    if (isPending) {
      if (canCountersign) {
        return (
          <Button size="sm" hasIcon onClick={() => actions.setDialog('countersign')}>
            <PenLine className="h-4 w-4" />
            Countersign
          </Button>
        )
      }
      return emailEnabled ? (
        <Button size="sm" hasIcon onClick={() => actions.openSend()}>
          <Send className="h-4 w-4" />
          Send a reminder
        </Button>
      ) : (
        <Explained reason="Quote emails are off">
          <Button size="sm" hasIcon disabled>
            <Send className="h-4 w-4" />
            Send a reminder
          </Button>
        </Explained>
      )
    }
    if (status === QuoteStatus.ACCEPTED) {
      return q.convertedToSubscriptionId ? (
        <Button size="sm" variant="outline" asChild>
          <Link to={`${basePath}/subscriptions/${q.convertedToSubscriptionId}`}>
            View subscription
          </Link>
        </Button>
      ) : (
        <Button
          size="sm"
          hasIcon
          disabled={actions.pending.convert}
          onClick={() => void actions.convert()}
        >
          <ArrowLeftRight className="h-4 w-4" />
          {actions.pending.convert ? 'Converting…' : 'Convert to subscription'}
        </Button>
      )
    }
    return null
  })()

  return (
    <header className="flex flex-col gap-3 border-b border-border bg-card px-6 py-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-1 basis-[360px] flex-col gap-1">
          <nav aria-label="Breadcrumb">
            <ol className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <li>
                <Link to={`${basePath}/quotes`} className="hover:text-foreground">
                  Quotes
                </Link>
              </li>
              <li aria-hidden="true">›</li>
              <li aria-current="page" className="text-foreground">
                {q.quoteNumber}
              </li>
            </ol>
          </nav>
          <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
            <h1 className="text-xl font-semibold text-foreground">{title}</h1>
            <QuoteStatusBadge status={status} />
            {q.revision > 1 && (
              <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
                Rev {q.revision}
              </span>
            )}
            {q.approvalStatus !== QuoteApprovalStatus.APPROVAL_NOT_REQUIRED && (
              <QuoteApprovalBadge status={q.approvalStatus} />
            )}
            {q.changesRequestedAt && <ChangesRequestedBadge />}
            {facts && <span className="text-[13px] text-muted-foreground">{facts}</span>}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {canSend && (
            <Button variant="outline" size="sm" hasIcon onClick={copyLink}>
              <CopyIcon className="h-4 w-4" />
              Copy link
            </Button>
          )}
          {canEdit && (
            <Button variant="outline" size="sm" hasIcon asChild>
              <Link to={`${basePath}/quotes/${q.id}/edit`}>
                <Pencil className="h-4 w-4" />
                Edit
              </Link>
            </Button>
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" aria-label="More actions">
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {isDraft && (
                <DropdownMenuItem
                  disabled={needsApproval || actions.pending.publish}
                  onClick={() => void actions.publish()}
                >
                  <FileText size="16" className="mr-2" />
                  Publish without sending
                </DropdownMenuItem>
              )}
              {isDraft && !emailEnabled && (
                <DropdownMenuItem disabled>
                  <Send size="16" className="mr-2" />
                  Send to customer (email off)
                </DropdownMenuItem>
              )}
              {isPending && canCountersign && (
                <DropdownMenuItem disabled={!emailEnabled} onClick={() => actions.openSend()}>
                  <Send size="16" className="mr-2" />
                  {emailEnabled ? 'Send a reminder' : 'Send a reminder (email off)'}
                </DropdownMenuItem>
              )}
              {canSend && (
                <DropdownMenuItem onClick={() => actions.setDialog('link')}>
                  <ExternalLink size="16" className="mr-2" />
                  Personal signing link
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onClick={() => actions.setDialog('preview')}>
                <Eye size="16" className="mr-2" />
                Preview as customer
              </DropdownMenuItem>
              {quote.pdfUrl && (
                <DropdownMenuItem asChild>
                  <a href={quote.pdfUrl} target="_blank" rel="noopener noreferrer">
                    <Download size="16" className="mr-2" />
                    {status === QuoteStatus.ACCEPTED ? 'Download signed PDF' : 'Download PDF'}
                  </a>
                </DropdownMenuItem>
              )}
              <DropdownMenuItem asChild>
                <Link to={`${basePath}/quotes/create?from=${q.id}`}>
                  <CopyPlus size="16" className="mr-2" />
                  Use as template
                </Link>
              </DropdownMenuItem>
              {canCancel && <DropdownMenuSeparator />}
              {isPending && (
                <DropdownMenuItem onClick={() => actions.setDialog('decline')}>
                  <ThumbsDown size="16" className="mr-2" />
                  Mark as declined
                </DropdownMenuItem>
              )}
              {canCancel && (
                <DropdownMenuItem
                  onClick={() => actions.setDialog('cancel')}
                  className="text-destructive focus:text-destructive"
                >
                  <XCircle size="16" className="mr-2" />
                  Cancel quote
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
          {primary}
        </div>
      </div>

      <QuoteProgress quote={quote} approval={approval} nameOf={nameOf} />
    </header>
  )
}
