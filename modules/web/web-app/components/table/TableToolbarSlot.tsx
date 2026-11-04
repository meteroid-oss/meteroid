import { ReactNode, createContext, useContext } from 'react'
import { createPortal } from 'react-dom'

// Lets a table (which owns the react-table instance, and therefore the
// column/density view-options) render those options into a toolbar defined by
// an ancestor layout (TableCard). The layout exposes a DOM node through this
// context; the table portals into it when present, otherwise falls back to
// rendering the options inline above itself.
const TableToolbarSlotContext = createContext<HTMLElement | null>(null)

export const TableToolbarSlotProvider = TableToolbarSlotContext.Provider

export const useTableToolbarSlot = () => useContext(TableToolbarSlotContext)

export const TableToolbarPortal = ({ children }: { children: ReactNode }) => {
  const slot = useTableToolbarSlot()
  if (!slot) return null
  return createPortal(children, slot)
}

// Same mechanism for the card's footer (saved-view tabs on the left, pagination on the right):
// when a TableCard has a footer, the table portals its pagination into it instead of rendering it
// below the rows.
const TableFooterSlotContext = createContext<HTMLElement | null>(null)

export const TableFooterSlotProvider = TableFooterSlotContext.Provider

export const useTableFooterSlot = () => useContext(TableFooterSlotContext)
