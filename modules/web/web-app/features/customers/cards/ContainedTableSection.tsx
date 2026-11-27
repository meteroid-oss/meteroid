import { Button, Tooltip, TooltipContent, TooltipTrigger, cn } from '@md/ui'
import { Plus } from 'lucide-react'
import { ReactNode, useState } from 'react'

import {
  TableFooterSlotProvider,
  TableToolbarSlotProvider,
} from '@/components/table/TableToolbarSlot'

/**
 * A table in a bordered box on a detail page: the table's hairlines (0.5px at white 10.2%, drawn
 * for the list cards) are lost on this surface, so whole-pixel dividers in the frame's colour, and
 * a tinted header row. Harmless on a box that holds no table.
 */
export const containedTable = cn(
  '[&_thead_th]:!border-b [&_thead_th]:!border-border [&_thead_th]:bg-foreground/[0.035]',
  '[&_tbody_tr:not(:last-child)>td]:!border-b [&_td]:!border-border [&_tbody_td]:!border-t-0'
)

/** A section's "New …" action: a quiet outlined icon button, named by its tooltip. */
export const SectionAddButton = ({ label, onClick }: { label: string; onClick: () => void }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <Button
        variant="ghost"
        size="sm"
        // A circle: a translucent outline over a fill a step lighter than the surface.
        className="size-7 rounded-full border border-foreground/10 bg-foreground/[0.04] px-0 text-muted-foreground hover:bg-foreground/[0.08] hover:text-foreground"
        onClick={onClick}
        aria-label={label}
      >
        <Plus size={14} />
      </Button>
    </TooltipTrigger>
    <TooltipContent>{label}</TooltipContent>
  </Tooltip>
)

/**
 * A titled table on the customer page, framed as the tables in Settings: a heading with the
 * table's paging, its view options and an optional "New …" action, then the rows in a bordered
 * box, with no footer.
 */
export const ContainedTableSection = ({
  title,
  actionLabel,
  onAction,
  headerExtra,
  children,
}: {
  title: string
  /** "New invoice", "New quote"…: the add button's tooltip. Left out without `onAction`. */
  actionLabel?: string
  onAction?: () => void
  /** More controls in the heading, before the paging (e.g. a period picker). */
  headerExtra?: ReactNode
  children: ReactNode
}) => {
  const [toolbarSlot, setToolbarSlot] = useState<HTMLDivElement | null>(null)
  const [pagingSlot, setPagingSlot] = useState<HTMLDivElement | null>(null)

  return (
    <section className="flex flex-col gap-4">
      <div className="flex h-7 items-center justify-between gap-2">
        <h3 className="text-base font-medium">{title}</h3>
        <div className="flex items-center gap-2">
          {headerExtra}
          {/* The table's pagination portals in here, as into a list card's footer. On a single
              page it is only a count ("4 results"), which the heading leaves out. */}
          <div ref={setPagingSlot} className="flex items-center [&>span]:hidden" />
          <div ref={setToolbarSlot} className="flex items-center" />
          {onAction && actionLabel && <SectionAddButton label={actionLabel} onClick={onAction} />}
        </div>
      </div>
      <div
        className={cn('overflow-hidden rounded-lg border border-border bg-card', containedTable)}
      >
        <TableToolbarSlotProvider value={toolbarSlot}>
          <TableFooterSlotProvider value={pagingSlot}>{children}</TableFooterSlotProvider>
        </TableToolbarSlotProvider>
      </div>
    </section>
  )
}
