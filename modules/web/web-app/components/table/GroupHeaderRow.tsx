import { TableCell, TableRow, cn } from '@md/ui'

import type { ReactNode } from 'react'

const HEADER = 'bg-muted/40 py-1.5 text-xs font-medium text-muted-foreground'

export const GroupHeaderRow = ({ colSpan, children }: { colSpan: number; children: ReactNode }) => (
  // `[&&]` outranks the table's row-height rules so the header stays compact in every density.
  <TableRow className="bg-muted/40 hover:bg-muted/40 [&&]:!h-8">
    <TableCell colSpan={colSpan} className={HEADER}>
      {children}
    </TableCell>
  </TableRow>
)

/** The same header over a divided list that is not a table. */
export const GroupHeader = ({ className, children }: { className?: string; children: ReactNode }) => (
  <div className={cn(HEADER, className)}>{children}</div>
)
