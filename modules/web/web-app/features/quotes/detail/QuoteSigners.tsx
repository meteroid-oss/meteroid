import {
  Badge,
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { BellRing, CopyIcon, Download, PenLine } from 'lucide-react'
import { ReactNode } from 'react'

import { Slot } from '@/edition'
import { RailCard } from '@/features/quotes/detail/RailCard'
import { fromNow, initials, shortDate } from '@/features/quotes/detail/quoteFormat'
import { sameEmail } from '@/features/quotes/detail/signers'
import {
  DetailedQuote,
  QuoteEngagement,
  QuoteSignatureLevel,
  QuoteSignerRole,
  QuoteStatus,
} from '@/rpc/api/quotes/v1/models_pb'
import { parseAndFormatDateTime, parseUtc } from '@/utils/date'

interface Props {
  quote: DetailedQuote
  currentUserEmail?: string
  /** Set when the current user can countersign now. */
  onCountersign?: () => void
  onCopySigningLink: (email: string) => void
  /** Emails that one person the link again; only offered while the quote awaits signatures. */
  onRemind: (person: { name: string; email: string }) => void
  onChanged: () => void
}

const RemindButton = ({ onClick }: { onClick: () => void }) => (
  <Button variant="ghost" size="sm" hasIcon className="h-7 px-2 text-xs" onClick={onClick}>
    <BellRing className="h-3 w-3" />
    Remind
  </Button>
)

const formatTimestamp = (value: string) => parseAndFormatDateTime(parseUtc(value).toISOString())

const distinctCount = (values: (string | undefined)[]) =>
  new Set(values.filter((v): v is string => Boolean(v))).size

const EngagementLine = ({
  engagement,
  email,
  published,
}: {
  engagement: QuoteEngagement
  email: string
  published: boolean
}) => {
  const viewed = engagement.recipients.find(r => sameEmail(r.email, email))
  if (!viewed) {
    return published ? <span className="text-xs text-muted-foreground">Not opened yet</span> : null
  }
  const label = `Viewed ${viewed.viewCount}× · last ${fromNow(viewed.lastViewedAt)} · rev ${viewed.lastViewedRevision}`
  const title = `First opened ${formatTimestamp(viewed.firstViewedAt)}, last opened ${formatTimestamp(viewed.lastViewedAt)}`
  const sessions = engagement.sessions.filter(s => sameEmail(s.email, email))
  if (sessions.length === 0) {
    return (
      <span className="text-xs text-foreground" title={title}>
        {label}
      </span>
    )
  }
  const devices = Math.max(
    distinctCount(sessions.map(s => s.ipAddress)),
    distinctCount(sessions.map(s => s.device))
  )
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            title={title}
            className="rounded-sm text-left text-xs text-foreground underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {label}
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80 p-0">
          <p className="border-b border-border px-3 py-2 text-xs font-medium text-foreground">
            Views
          </p>
          <ul aria-label={`Views by ${email}`} className="max-h-72 overflow-y-auto py-1">
            {sessions.map((session, index) => (
              <li key={index} className="flex flex-col gap-0.5 px-3 py-1.5 text-xs">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="tabular-nums text-foreground">
                    {formatTimestamp(session.viewedAt)}
                  </span>
                  <span className="shrink-0 text-muted-foreground">rev {session.revision}</span>
                </div>
                <span className="truncate text-muted-foreground">
                  {[session.ipAddress, session.device].filter(Boolean).join(' · ') || '—'}
                </span>
              </li>
            ))}
          </ul>
        </PopoverContent>
      </Popover>
      {devices >= 2 && (
        <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
          {devices} devices
        </Badge>
      )}
    </div>
  )
}

const SignerRow = ({
  name,
  email,
  you,
  role,
  status,
  engagement,
  actions,
}: {
  name: string
  email: string
  you: boolean
  role: string
  status: ReactNode
  engagement?: ReactNode
  actions?: ReactNode
}) => (
  <li className="flex items-start gap-3 border-t border-border pt-3 first:border-t-0 first:pt-0">
    <span
      aria-hidden="true"
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold text-muted-foreground"
    >
      {initials(name, email)}
    </span>
    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
      <div className="flex items-start justify-between gap-2">
        <span className="truncate text-sm font-medium text-foreground">
          {name || email}
          {you && <span className="font-normal text-muted-foreground"> (you)</span>}
        </span>
        {status}
      </div>
      <span className="truncate text-xs text-muted-foreground">
        {email} · {role}
      </span>
      {engagement}
      {actions && <div className="mt-1 flex flex-wrap gap-1">{actions}</div>}
    </div>
  </li>
)

export const QuoteSigners = ({
  quote,
  currentUserEmail,
  onCountersign,
  onCopySigningLink,
  onRemind,
  onChanged,
}: Props) => {
  const q = quote.quote
  if (!q) return null
  const published = Boolean(q.publishedAt) || q.status !== QuoteStatus.DRAFT
  const awaitingSignatures = q.status === QuoteStatus.PENDING
  const canRemind = awaitingSignatures && quote.quoteEmailEnabled !== false
  const signatureOf = (email: string, role: QuoteSignerRole) =>
    quote.signatures.find(s => s.signerRole === role && sameEmail(s.signedByEmail, email))
  const customersSigned =
    q.recipients.length > 0 &&
    q.recipients.every(r => signatureOf(r.email, QuoteSignerRole.SIGNER_CUSTOMER))
  const signedPdfUrl = q.status === QuoteStatus.ACCEPTED ? quote.pdfUrl : undefined

  const signedBadge = (signedAt: string) => (
    <Badge
      variant="success"
      size="sm"
      className="shrink-0"
      title={parseAndFormatDateTime(parseUtc(signedAt).toISOString())}
    >
      Signed · {shortDate(signedAt)}
    </Badge>
  )

  return (
    <RailCard
      title="Signers"
      aside={
        q.signatureLevel === QuoteSignatureLevel.VERIFIED && (
          <Badge variant="outline" size="sm">
            Verified
          </Badge>
        )
      }
    >
      {q.recipients.length === 0 && q.countersigners.length === 0 ? (
        <p className="text-sm text-muted-foreground">No signers</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {q.recipients.map((recipient, index) => {
            const signature = signatureOf(recipient.email, QuoteSignerRole.SIGNER_CUSTOMER)
            return (
              <SignerRow
                key={`r-${index}`}
                name={recipient.name}
                email={recipient.email}
                you={sameEmail(recipient.email, currentUserEmail)}
                role="Customer"
                status={
                  signature ? (
                    signedBadge(signature.signedAt)
                  ) : (
                    <Badge
                      variant={awaitingSignatures ? 'warning' : 'outline'}
                      size="sm"
                      className="shrink-0"
                    >
                      {awaitingSignatures ? 'To sign' : 'Not signed'}
                    </Badge>
                  )
                }
                engagement={
                  quote.engagement && (
                    <EngagementLine
                      engagement={quote.engagement}
                      email={recipient.email}
                      published={published}
                    />
                  )
                }
                actions={
                  !signature && (
                    <>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="outline"
                            size="sm"
                            hasIcon
                            className="h-7 px-2 text-xs"
                            onClick={() => onCopySigningLink(recipient.email)}
                          >
                            <CopyIcon className="h-3 w-3" />
                            Copy link
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent className="max-w-xs">
                          Anyone with this link can sign as {recipient.name || recipient.email}
                        </TooltipContent>
                      </Tooltip>
                      {canRemind && <RemindButton onClick={() => onRemind(recipient)} />}
                    </>
                  )
                }
              />
            )
          })}
          {q.countersigners.map((countersigner, index) => {
            const signature = signatureOf(countersigner.email, QuoteSignerRole.SIGNER_COUNTERSIGNER)
            const you = sameEmail(countersigner.email, currentUserEmail)
            return (
              <SignerRow
                key={`c-${index}`}
                name={countersigner.name}
                email={countersigner.email}
                you={you}
                role={
                  countersigner.title ? `Countersigner, ${countersigner.title}` : 'Countersigner'
                }
                status={
                  signature ? (
                    signedBadge(signature.signedAt)
                  ) : countersigner.automatic ? (
                    <Badge variant="outline" size="sm" className="shrink-0">
                      Automatic
                    </Badge>
                  ) : (
                    <Badge
                      variant={awaitingSignatures && customersSigned ? 'warning' : 'outline'}
                      size="sm"
                      className="shrink-0"
                    >
                      {awaitingSignatures && customersSigned ? 'To countersign' : 'Countersigns'}
                    </Badge>
                  )
                }
                engagement={
                  quote.engagement && (
                    <EngagementLine
                      engagement={quote.engagement}
                      email={countersigner.email}
                      published={false}
                    />
                  )
                }
                actions={
                  you && onCountersign ? (
                    <Button size="sm" hasIcon className="h-7 px-2 text-xs" onClick={onCountersign}>
                      <PenLine className="h-3 w-3" />
                      Countersign
                    </Button>
                  ) : (
                    !you &&
                    !signature &&
                    !countersigner.automatic &&
                    customersSigned &&
                    canRemind && <RemindButton onClick={() => onRemind(countersigner)} />
                  )
                }
              />
            )
          })}
        </ul>
      )}

      {signedPdfUrl && (
        <div className="flex items-center justify-between gap-2 border-t border-border pt-3">
          <Button variant="outline" size="sm" hasIcon asChild>
            <a href={signedPdfUrl} target="_blank" rel="noopener noreferrer">
              <Download className="h-4 w-4" />
              Download signed PDF
            </a>
          </Button>
          <Slot name="quoteDetail.certificate" quote={quote} onChanged={onChanged} />
        </div>
      )}
    </RailCard>
  )
}
