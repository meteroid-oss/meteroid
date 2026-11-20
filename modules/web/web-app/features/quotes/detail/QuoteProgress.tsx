import { cn } from '@md/ui'
import { Check, X } from 'lucide-react'
import { Fragment } from 'react'

import { fromNow, shortDate } from '@/features/quotes/detail/quoteFormat'
import { hasSignedAs } from '@/features/quotes/detail/signers'
import {
  DetailedQuote,
  QuoteApprovalStatus,
  QuoteSignerRole,
  QuoteStatus,
} from '@/rpc/api/quotes/v1/models_pb'
import { GetQuoteApprovalResponse } from '@/rpc/api/quotes/v1/quotes_pb'

type StepState = 'done' | 'current' | 'upcoming' | 'stopped'

interface Step {
  id: string
  label: string
  done: boolean
  detail?: string
}

const latestSignedAt = (quote: DetailedQuote, role: QuoteSignerRole) =>
  quote.signatures
    .filter(s => s.signerRole === role)
    .map(s => s.signedAt)
    .sort()
    .at(-1)

const terminal = (quote: DetailedQuote): { label: string; at?: string } | undefined => {
  const q = quote.quote
  switch (q?.status) {
    case QuoteStatus.DECLINED:
      return { label: 'Declined', at: q.declinedAt }
    case QuoteStatus.EXPIRED:
      return { label: 'Expired', at: q.expiresAt }
    case QuoteStatus.CANCELLED:
      return { label: 'Cancelled', at: q.updatedAt }
    default:
      return undefined
  }
}

export const quoteProgressSteps = (
  quote: DetailedQuote,
  approval: GetQuoteApprovalResponse | undefined,
  nameOf: (userId?: string) => string
): (Step & { state: StepState })[] => {
  const q = quote.quote
  if (!q) return []
  const isDraft = q.status === QuoteStatus.DRAFT
  const steps: Step[] = [
    { id: 'created', label: 'Created', done: true, detail: shortDate(q.createdAt) },
  ]

  const latest = approval?.latest
  const approvalRequired =
    (approval?.matchedRules.length ?? 0) > 0 ||
    latest !== undefined ||
    q.approvalStatus !== QuoteApprovalStatus.APPROVAL_NOT_REQUIRED
  if (approvalRequired) {
    const approved = !isDraft || approval?.satisfied === true
    const decided =
      latest?.status === QuoteApprovalStatus.APPROVAL_APPROVED && latest.decidedAt
        ? `${nameOf(latest.decidedBy)} · ${shortDate(latest.decidedAt)}`
        : undefined
    steps.push({
      id: 'approved',
      label: 'Approved',
      done: approved,
      detail: approved
        ? decided
        : latest?.status === QuoteApprovalStatus.APPROVAL_PENDING
          ? 'Pending'
          : latest?.status === QuoteApprovalStatus.APPROVAL_REJECTED
            ? 'Rejected'
            : undefined,
    })
  }

  const sentAt = q.lastSentAt ?? q.publishedAt
  steps.push({
    id: 'sent',
    label: q.lastSentAt || !q.publishedAt ? 'Sent' : 'Published',
    done: !isDraft || Boolean(sentAt),
    detail: sentAt ? shortDate(sentAt) : undefined,
  })

  const views = quote.engagement?.recipients ?? []
  const viewCount = views.reduce((sum, r) => sum + r.viewCount, 0)
  const lastViewedAt = views
    .map(r => r.lastViewedAt)
    .sort()
    .at(-1)
  steps.push({
    id: 'viewed',
    label: 'Viewed',
    done: Boolean(q.firstViewedAt) || viewCount > 0,
    detail:
      viewCount > 0 && lastViewedAt
        ? `${viewCount}× · ${fromNow(lastViewedAt)}`
        : q.firstViewedAt
          ? shortDate(q.firstViewedAt)
          : undefined,
  })

  const customersSigned =
    q.status === QuoteStatus.ACCEPTED ||
    (q.recipients.length > 0 &&
      q.recipients.every(r =>
        hasSignedAs(quote.signatures, r.email, QuoteSignerRole.SIGNER_CUSTOMER)
      ))
  const customerSignedAt = latestSignedAt(quote, QuoteSignerRole.SIGNER_CUSTOMER)
  steps.push({
    id: 'signed',
    label: 'Customer signature',
    done: customersSigned,
    detail: customersSigned && customerSignedAt ? shortDate(customerSignedAt) : undefined,
  })

  if (q.countersigners.length > 0) {
    const countersigned =
      q.status === QuoteStatus.ACCEPTED ||
      q.countersigners.every(c =>
        hasSignedAs(quote.signatures, c.email, QuoteSignerRole.SIGNER_COUNTERSIGNER)
      )
    const countersignedAt = latestSignedAt(quote, QuoteSignerRole.SIGNER_COUNTERSIGNER)
    steps.push({
      id: 'countersigned',
      label: 'Countersignature',
      done: countersigned,
      detail: countersigned && countersignedAt ? shortDate(countersignedAt) : undefined,
    })
  }

  steps.push({
    id: 'subscription',
    label: 'Subscription',
    done: Boolean(q.convertedToSubscriptionId),
    detail: q.convertedAt ? shortDate(q.convertedAt) : undefined,
  })

  // A later step proves the earlier ones, e.g. a signature recorded before views were tracked.
  for (let i = steps.length - 2; i >= 0; i--) {
    if (steps[i + 1].done) steps[i].done = true
  }

  const stop = terminal(quote)
  const currentIndex = steps.findIndex(step => !step.done)
  return steps.map((step, index) => {
    if (index !== currentIndex) {
      return { ...step, state: step.done ? 'done' : 'upcoming' }
    }
    return stop
      ? {
          ...step,
          label: stop.label,
          detail: stop.at ? shortDate(stop.at) : undefined,
          state: 'stopped',
        }
      : { ...step, state: 'current' }
  })
}

interface Props {
  quote: DetailedQuote
  approval?: GetQuoteApprovalResponse
  nameOf: (userId?: string) => string
}

export const QuoteProgress = ({ quote, approval, nameOf }: Props) => {
  const steps = quoteProgressSteps(quote, approval, nameOf)

  return (
    <ol
      aria-label="Quote progress"
      className="flex flex-wrap items-center gap-x-1.5 gap-y-2 text-xs"
    >
      {steps.map((step, index) => (
        <Fragment key={step.id}>
          {index > 0 && (
            <li aria-hidden="true" className="text-muted-foreground/60">
              —
            </li>
          )}
          <li
            className="flex items-center gap-1.5"
            aria-current={step.state === 'current' || step.state === 'stopped' ? 'step' : undefined}
          >
            <span
              className={cn(
                'inline-flex h-6 items-center gap-1 rounded-full px-2 font-medium',
                step.state === 'done' && 'bg-muted text-foreground',
                step.state === 'current' && 'bg-accent text-foreground ring-1 ring-primary/40',
                step.state === 'upcoming' &&
                  'border border-dashed border-border text-muted-foreground',
                step.state === 'stopped' && 'bg-destructive/10 text-destructive'
              )}
            >
              {step.state === 'done' && <Check className="h-3.5 w-3.5 text-success" aria-hidden />}
              {step.state === 'stopped' && <X className="h-3.5 w-3.5" aria-hidden />}
              {step.label}
              {step.state === 'done' && <span className="sr-only">(done)</span>}
            </span>
            {step.detail && <span className="text-muted-foreground">{step.detail}</span>}
          </li>
        </Fragment>
      ))}
    </ol>
  )
}
