import { Button, cn } from '@md/ui'
import { XIcon } from 'lucide-react'

import type { ReactNode } from 'react'

/** A catalog list with its selected row open beside it, on the same page. */
export const ListWithPanel = ({ panel, children }: { panel?: ReactNode; children: ReactNode }) => (
  <div className="flex flex-col gap-4 xl:flex-row xl:items-start">
    <div className="min-w-0 flex-1">{children}</div>
    {panel && (
      <aside
        className={cn(
          'w-full shrink-0 rounded-md border border-border bg-card',
          // Sticky beside the list on wide screens; under it otherwise.
          'xl:sticky xl:top-4 xl:max-h-[calc(100vh-2rem)] xl:w-[460px] xl:overflow-y-auto'
        )}
      >
        {panel}
      </aside>
    )}
  </div>
)

/** The panel's head: what is open, a note on how edits apply, and its close. */
export const PanelHead = ({
  title,
  note,
  actions,
  onClose,
}: {
  title: ReactNode
  note?: ReactNode
  actions?: ReactNode
  onClose: () => void
}) => (
  <div className="flex items-start gap-2 border-b border-border px-4 py-3">
    <div className="min-w-0 flex-1">{title}</div>
    {note && <span className="pt-1 text-xs text-muted-foreground">{note}</span>}
    {actions}
    <Button
      variant="ghost"
      size="icon"
      className="size-7 shrink-0 text-muted-foreground"
      onClick={onClose}
      aria-label="Close"
    >
      <XIcon size={14} />
    </Button>
  </div>
)

/** A titled block of a panel. */
export const PanelSection = ({
  title,
  hint,
  children,
  className,
}: {
  title: ReactNode
  hint?: ReactNode
  children: ReactNode
  className?: string
}) => (
  <section className={cn('space-y-2 px-4 py-3', className)}>
    <div>
      <h3 className="text-sm font-medium">{title}</h3>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
    {children}
  </section>
)
