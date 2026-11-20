import { Alert, Button } from '@md/ui'
import { format } from 'date-fns'
import { Check, CloudOff, Loader2 } from 'lucide-react'

import { useMembers } from '@/features/accounts/hooks/useMembers'
import { parseUtc } from '@/utils/date'

import type { AutosaveStatus, RestoreOffer } from '@/features/quotes/editor/useQuoteAutosave'
import type { QuoteDraft } from '@/rpc/api/quotes/v1/models_pb'

const formatTime = (date: Date) => format(date, 'MMM d, HH:mm')

export const AutosaveIndicator = ({ status }: { status: AutosaveStatus }) => {
  switch (status) {
    case 'saving':
      return (
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          Saving…
        </span>
      )
    case 'saved':
      return (
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <Check className="h-3 w-3" />
          Saved
        </span>
      )
    case 'local':
      return (
        <span
          className="flex items-center gap-1 text-xs text-warning"
          title="The server could not be reached: your changes are kept in this browser."
        >
          <CloudOff className="h-3 w-3" />
          Kept on this device
        </span>
      )
    default:
      return null
  }
}

export const DraftConflictBanner = ({
  draft,
  onLoadTheirs,
  onKeepMine,
}: {
  draft: QuoteDraft
  onLoadTheirs: () => void
  onKeepMine: () => void
}) => {
  const { byId, meId } = useMembers()
  const author = draft.updatedBy
    ? draft.updatedBy === meId
      ? 'you, in another tab'
      : (byId.get(draft.updatedBy)?.displayName ?? 'someone else')
    : 'someone else'

  return (
    <Alert variant="warning" className="flex flex-wrap items-center justify-between gap-2">
      <span>
        Changed elsewhere (by {author}, at {formatTime(parseUtc(draft.updatedAt))}). Your changes
        are not saved until you choose.
      </span>
      <div className="flex gap-2">
        <Button type="button" size="sm" variant="outline" onClick={onLoadTheirs}>
          Load their version
        </Button>
        <Button type="button" size="sm" onClick={onKeepMine}>
          Keep mine
        </Button>
      </div>
    </Alert>
  )
}

/** The quote was saved by someone else since this editor loaded it: never blocking. */
export const StaleQuoteBanner = ({
  savedVersionHref,
  onOverwrite,
}: {
  savedVersionHref: string
  onOverwrite: () => void
}) => (
  <Alert variant="warning" className="flex flex-wrap items-center justify-between gap-2">
    <span>
      This quote was saved by someone else since you opened it. Saving replaces their changes.
    </span>
    <div className="flex gap-2">
      <Button type="button" size="sm" variant="outline" asChild>
        <a href={savedVersionHref} target="_blank" rel="noreferrer">
          Open the saved version
        </a>
      </Button>
      <Button type="button" size="sm" onClick={onOverwrite}>
        Overwrite
      </Button>
    </div>
  </Alert>
)

export const DraftRestoreBanner = ({
  offer,
  onRestore,
  onDiscard,
}: {
  offer: RestoreOffer
  onRestore: () => void
  onDiscard: () => void
}) => (
  <Alert className="flex flex-wrap items-center justify-between gap-2">
    <span>
      Restore unsaved changes from {formatTime(offer.savedAt)}
      {offer.source === 'device' ? ', kept on this device' : ''}?
    </span>
    <div className="flex gap-2">
      <Button type="button" size="sm" variant="outline" onClick={onDiscard}>
        Discard
      </Button>
      <Button type="button" size="sm" onClick={onRestore}>
        Restore
      </Button>
    </div>
  </Alert>
)
