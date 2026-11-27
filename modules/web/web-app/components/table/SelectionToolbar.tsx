import { cn } from '@md/ui'
import { menuSurface } from '@ui/components/ui/menu-styles'
import { XIcon } from 'lucide-react'
import { ReactNode, useRef } from 'react'
import { createPortal } from 'react-dom'

/**
 * The selected rows' count and bulk actions, floating at the bottom of the table so the rows
 * don't move when a selection starts or ends. Inside a TableCard it sits just above the card's
 * footer, centred on the card; elsewhere at the bottom of the window.
 *
 * It stays mounted, so it can fold away again (see `.mt-selection-bar` in main.css); while
 * closed it is hidden from Tab and screen readers.
 */
export const SelectionToolbar = ({
  count,
  footerSlot,
  onClear,
  children,
}: {
  count: number
  /** The TableCard footer's slot, when the table sits in one. */
  footerSlot: HTMLElement | null
  onClear: () => void
  children?: ReactNode
}) => {
  const open = count > 0
  // While it folds away, keep showing what was selected instead of "0 selected" and no actions.
  const last = useRef<{ count: number; children?: ReactNode }>({ count: 0 })
  if (open) last.current = { count, children }
  const shown = last.current

  const bar = (
    <div
      className={cn(
        'mt-selection-bar pointer-events-none z-20 flex justify-center',
        footerSlot ? 'absolute inset-x-0 bottom-full mb-3' : 'fixed inset-x-0 bottom-6'
      )}
      data-open={open ? 'true' : 'false'}
    >
      <div
        role="toolbar"
        aria-label="Selected rows"
        className={cn(
          menuSurface,
          'mt-selection-bar-surface pointer-events-auto flex max-w-[calc(100%-2rem)] items-center gap-1 py-1 pl-3 pr-1 text-[13px]'
        )}
      >
        <span
          className="whitespace-nowrap font-medium tabular-nums text-foreground"
          aria-live="polite"
        >
          {shown.count} selected
        </span>
        <span aria-hidden className="mx-2 h-4 w-px shrink-0 bg-foreground/12" />
        <div className="flex min-w-0 items-center gap-2">{shown.children}</div>
        <button
          type="button"
          onClick={onClear}
          aria-label="Clear selection"
          className="ml-1 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-foreground/[0.08] hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <XIcon size={14} />
        </button>
      </div>
    </div>
  )

  // The footer is sticky, so the bar rides with it at the window's bottom on long lists.
  return createPortal(bar, footerSlot?.parentElement ?? document.body)
}
