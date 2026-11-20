import { Button, cn } from '@md/ui'
import { AlertCircle, Check, Download, Hourglass, Pen, X } from 'lucide-react'
import { ReactNode } from 'react'

export type QuoteOutcome =
  | { kind: 'declined'; reason?: string }
  | { kind: 'accepted'; signedPdfUrl?: string }
  | { kind: 'signed'; awaitingCountersignature: boolean }
  | { kind: 'cancelled' }
  | { kind: 'expired'; on?: string }
  | { kind: 'draft' }

interface Props {
  /** e.g. "€2,235.00 / month · 12 months" */
  summary?: string
  outcome?: QuoteOutcome
  actions?: {
    signLabel: string
    onSign: () => void
    onRequestChanges?: () => void
    onDecline?: () => void
    disabled: boolean
    signDisabled: boolean
  }
}

const State = ({ icon, children }: { icon: ReactNode; children: ReactNode }) => (
  <div role="status" className="flex items-start gap-2 text-sm text-foreground">
    <span aria-hidden className="mt-0.5 shrink-0">
      {icon}
    </span>
    <div className="min-w-0">{children}</div>
  </div>
)

const OutcomeState = ({ outcome }: { outcome: QuoteOutcome }) => {
  switch (outcome.kind) {
    case 'declined':
      return (
        <State icon={<X className="h-4 w-4 text-destructive" />}>
          <p className="font-medium">Declined</p>
          {outcome.reason && <p className="break-words text-muted-foreground">{outcome.reason}</p>}
        </State>
      )
    case 'accepted':
      return (
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <State icon={<Check className="h-4 w-4 text-success" />}>
            <p className="font-medium">Signed and accepted</p>
          </State>
          {outcome.signedPdfUrl && (
            <Button variant="outline" hasIcon asChild>
              <a href={outcome.signedPdfUrl} target="_blank" rel="noopener noreferrer">
                <Download className="h-4 w-4" />
                Download signed PDF
              </a>
            </Button>
          )}
        </div>
      )
    case 'signed':
      return outcome.awaitingCountersignature ? (
        <State icon={<Hourglass className="h-4 w-4 text-muted-foreground" />}>
          <p className="font-medium">Signed · awaiting countersignature</p>
        </State>
      ) : (
        <State icon={<Check className="h-4 w-4 text-success" />}>
          <p className="font-medium">Signed · awaiting other signers</p>
        </State>
      )
    case 'cancelled':
      return (
        <State icon={<AlertCircle className="h-4 w-4 text-muted-foreground" />}>
          <p className="font-medium">Cancelled</p>
        </State>
      )
    case 'expired':
      return (
        <State icon={<AlertCircle className="h-4 w-4 text-destructive" />}>
          <p className="font-medium">{outcome.on ? `Expired on ${outcome.on}` : 'Expired'}</p>
        </State>
      )
    case 'draft':
      return (
        <State icon={<AlertCircle className="h-4 w-4 text-muted-foreground" />}>
          <p className="font-medium">Not published</p>
        </State>
      )
  }
}

/** Sticky footer: what the quote costs, and what the viewer can do with it. */
export const QuoteActionBar = ({ summary, outcome, actions }: Props) => {
  if (!summary && !outcome && !actions) return null

  return (
    <footer
      className={cn(
        'sticky bottom-0 z-20 border-t border-border bg-card',
        // The summary alone is shown at the top of the page on phones.
        !outcome && !actions && 'hidden sm:block'
      )}
    >
      <div className="mx-auto flex w-full max-w-[1320px] flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:px-6">
        {summary && (
          <div className="hidden min-w-0 flex-1 flex-col sm:flex">
            <p className="truncate font-semibold text-foreground">{summary}</p>
          </div>
        )}

        {outcome && (
          <div className={summary ? 'sm:max-w-[60%]' : 'flex-1'}>
            <OutcomeState outcome={outcome} />
          </div>
        )}

        {actions && (
          <div className="flex flex-col-reverse gap-2 sm:ml-auto sm:flex-row sm:items-center">
            {(actions.onDecline || actions.onRequestChanges) && (
              <div className="flex gap-2">
                {actions.onDecline && (
                  <Button
                    type="button"
                    variant="ghost"
                    className="h-10 flex-1 sm:flex-none"
                    onClick={actions.onDecline}
                    disabled={actions.disabled}
                  >
                    Decline
                  </Button>
                )}
                {actions.onRequestChanges && (
                  <Button
                    type="button"
                    variant="outline"
                    className="h-10 flex-1 sm:flex-none"
                    onClick={actions.onRequestChanges}
                  >
                    Request changes
                  </Button>
                )}
              </div>
            )}
            <Button
              type="button"
              variant="brand"
              hasIcon
              className="h-10 w-full sm:w-auto"
              onClick={actions.onSign}
              disabled={actions.disabled || actions.signDisabled}
            >
              <Pen aria-hidden className="h-4 w-4" />
              {actions.signLabel}
            </Button>
          </div>
        )}
      </div>
    </footer>
  )
}
