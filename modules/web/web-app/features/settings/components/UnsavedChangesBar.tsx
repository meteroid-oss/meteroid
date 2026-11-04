import { Button, cn } from '@md/ui'

interface Props {
  show: boolean
  saving?: boolean
  onDiscard: () => void
  className?: string
}

// Floating save pill for long settings forms; render it inside the <form> so Save submits it.
// It sticks to the bottom of the scroll area, so Save is reachable wherever the edit happened.
export const UnsavedChangesBar = ({ show, saving, onDiscard, className }: Props) => {
  if (!show) return null
  return (
    <div
      className={cn('pointer-events-none sticky bottom-4 z-20 flex justify-center', className)}
      role="region"
      aria-label="Unsaved changes"
    >
      <div className="pointer-events-auto flex items-center gap-2 rounded-full border border-border bg-popover py-1.5 pl-4 pr-1.5 shadow-lg">
        <span className="text-[13px] text-muted-foreground">Unsaved changes</span>
        <Button type="button" variant="ghost" size="sm" onClick={onDiscard} disabled={saving}>
          Discard
        </Button>
        <Button type="submit" size="sm" disabled={saving}>
          {saving ? 'Saving…' : 'Save'}
        </Button>
      </div>
    </div>
  )
}
