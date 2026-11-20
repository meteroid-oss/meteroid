import { Badge, Button } from '@md/ui'
import { Download, MessageSquare } from 'lucide-react'

import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { CommentPin } from '@/features/quotes/detail/QuoteLinesTable'
import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'

interface Props {
  sellerName: string
  logoUrl?: string
  quoteNumber?: string
  customerName?: string
  viewerName: string
  viewerEmail: string
  status?: QuoteStatus
  /** The quote's PDF; once accepted, the action bar offers the signed one. */
  pdfUrl?: string
  expired: boolean
  /** Whole days left before the quote expires, while it is pending. */
  daysLeft?: number
  questions?: { count: number; open: boolean; onToggle: () => void }
}

const expiryLabel = (days: number) =>
  days <= 0 ? 'Expires today' : days === 1 ? 'Expires tomorrow' : `Expires in ${days} days`

export const QuotePortalTopBar = ({
  sellerName,
  logoUrl,
  quoteNumber,
  customerName,
  viewerName,
  viewerEmail,
  status,
  pdfUrl,
  expired,
  daysLeft,
  questions,
}: Props) => (
  <header className="border-b border-border bg-card">
    <div className="mx-auto flex w-full max-w-[1320px] items-center gap-3 px-4 py-3 sm:px-6">
      {logoUrl ? (
        <img
          src={logoUrl}
          alt={`${sellerName} logo`}
          className="h-8 w-auto max-w-[96px] shrink-0 object-contain sm:max-w-[120px]"
        />
      ) : (
        <span
          aria-hidden
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted text-sm font-semibold text-foreground"
        >
          {sellerName.charAt(0).toUpperCase()}
        </span>
      )}

      <div className="flex min-w-0 flex-1 flex-col">
        <h1 className="truncate text-sm font-semibold text-foreground sm:text-base">
          Quote {quoteNumber}
          {customerName && ` for ${customerName}`}
        </h1>
        {viewerEmail && (
          <p className="truncate text-xs text-muted-foreground">
            Viewing as {viewerName || viewerEmail}
            {viewerName && ` (${viewerEmail})`}
          </p>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {expired ? (
          <Badge variant="destructive" size="sm">
            Expired
          </Badge>
        ) : status === QuoteStatus.PENDING ? (
          daysLeft !== undefined && (
            <Badge variant={daysLeft <= 7 ? 'warning' : 'secondary'} size="sm">
              {expiryLabel(daysLeft)}
            </Badge>
          )
        ) : (
          status !== undefined && <QuoteStatusBadge status={status} />
        )}

        {pdfUrl && (
          <Button variant="outline" size="sm" hasIcon asChild className="mt-0">
            <a href={pdfUrl} target="_blank" rel="noopener noreferrer" aria-label="Download PDF">
              <Download aria-hidden className="h-4 w-4" />
              <span className="hidden sm:inline">Download PDF</span>
            </a>
          </Button>
        )}

        {questions && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            hasIcon
            aria-pressed={questions.open}
            aria-label={`Questions and changes, ${questions.count} ${questions.count === 1 ? 'message' : 'messages'}`}
            onClick={questions.onToggle}
            className="relative mt-0"
          >
            <MessageSquare aria-hidden className="h-4 w-4" />
            <span className="hidden sm:inline">
              Questions{questions.count > 0 && ` · ${questions.count}`}
            </span>
            {questions.count > 0 && (
              <CommentPin count={questions.count} className="absolute -right-2 -top-2 sm:hidden" />
            )}
          </Button>
        )}
      </div>
    </div>
  </header>
)
