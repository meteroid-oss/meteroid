import { Button, cn } from '@md/ui'
import { ChevronRight, RefreshCwIcon, XIcon } from 'lucide-react'
import { Fragment, ReactNode, createContext, useContext, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Link } from 'react-router'

import { SearchIcon } from '@/components/icons/AppIcons'
import { usePageIcon } from '@/components/layouts/TenantLayout/PageIconContext'
import {
  TableFooterSlotProvider,
  TableToolbarSlotProvider,
} from '@/components/table/TableToolbarSlot'

export interface Crumb {
  label: ReactNode
  to?: string
}

interface ListPageHeaderProps {
  title: ReactNode
  breadcrumbs?: Crumb[]
  tabs?: ReactNode
  actions?: ReactNode
}

/**
 * Compact page header shared by every list page: the section's icon and breadcrumb on
 * the left (the item count sits in the table's first column header), optional centered tabs, and optional actions on the right.
 */
export const ListPageHeader = ({ title, breadcrumbs, tabs, actions }: ListPageHeaderProps) => {
  const PageIcon = usePageIcon()

  return (
    <div className="flex h-11 shrink-0 items-center gap-2 pl-4 pr-3">
      <div className="flex flex-1 items-center gap-1 min-w-0 text-[13px] font-medium text-muted-foreground">
        {/* The icon of the nav section this page is in. */}
        <span
          aria-hidden
          className="flex size-5 shrink-0 items-center justify-center text-faint-foreground [&>svg]:max-h-4 [&>svg]:max-w-4"
        >
          <PageIcon />
        </span>
        {breadcrumbs?.map((crumb, i) => (
          <Fragment key={i}>
            <ChevronRight size={14} className="mx-px shrink-0 text-faint-foreground" />
            {crumb.to ? (
              <Link to={crumb.to} className="truncate hover:text-foreground transition-colors">
                {crumb.label}
              </Link>
            ) : (
              <span className="truncate">{crumb.label}</span>
            )}
          </Fragment>
        ))}
        <ChevronRight size={14} className="mx-px shrink-0 text-faint-foreground" />
        <span className="flex items-center gap-1.5 p-1 leading-none truncate">
          <span className="truncate">{title}</span>
        </span>
      </div>
      {tabs && <div className="shrink-0">{tabs}</div>}
      <div className="flex flex-1 items-center justify-end gap-2">{actions}</div>
    </div>
  )
}

interface TableCardProps {
  /** Left side of the footer that sticks to the bottom of the window — e.g. the saved-view tabs.
   *  The table's pagination always sits on the footer's right side. */
  footer?: ReactNode
  /** Left side of the card toolbar — typically search + filters. */
  toolbar?: ReactNode
  /** Right side of the card toolbar — e.g. a refresh button. Table view-options
   *  (columns/density) portal in here automatically. */
  toolbarTrailing?: ReactNode
  children: ReactNode
  className?: string
}

/**
 * Full-width card wrapping a table and its toolbar, giving every list its
 * consistent card surface. It runs to the window's edges: 12px from the sidebar,
 * 8px from the right, open at the bottom.
 */
export const TableCard = ({
  footer,
  toolbar,
  toolbarTrailing,
  children,
  className,
}: TableCardProps) => {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null)
  const [searchSlot, setSearchSlot] = useState<HTMLDivElement | null>(null)
  const [footerSlot, setFooterSlot] = useState<HTMLDivElement | null>(null)
  const hasToolbar = toolbar !== undefined || toolbarTrailing !== undefined
  const hasFooter = footer !== undefined

  return (
    // The card fills the window down to its bottom edge (`data-fit-viewport` holds the page to the
    // window's height, see TenantPageLayout): the toolbar and footer stay put, and the rows scroll
    // inside the table, under its sticky column headers. `overflow-clip` rounds the corners.
    <div
      data-fit-viewport
      className={cn(
        'mt-table-card mx-3 flex min-h-0 flex-1 flex-col rounded-t-md border-[0.5px] border-b-0 border-foreground/12 bg-card-table overflow-clip',
        className
      )}
    >
      {hasToolbar && (
        // 44px: 30px controls with 7px above and below (taller only when the filter chips wrap).
        // The Filter button opens the row 8px from the card's edge.
        <div className="flex items-center justify-between gap-2 pl-2 pr-4 min-h-11 py-[7px] border-b-[0.5px] border-foreground/[0.102] shrink-0">
          <ToolbarSearchSlotContext.Provider value={searchSlot}>
            <div className="flex flex-1 flex-wrap items-center gap-1 min-w-0">{toolbar}</div>
          </ToolbarSearchSlotContext.Provider>
          <div className="flex items-center gap-1 shrink-0">
            {/* The search sits first among the actions on the right (see ListToolbarSearch). */}
            <div ref={setSearchSlot} className="contents" />
            {toolbarTrailing}
            <div ref={setSlot} className="flex items-center" />
          </div>
        </div>
      )}
      <TableToolbarSlotProvider value={slot}>
        <TableFooterSlotProvider value={footerSlot}>
          <div className="flex flex-1 flex-col min-w-0 min-h-0 [&_thead_th]:sticky [&_thead_th]:top-0 [&_thead_th]:z-[1] [&_thead_th]:bg-card-table">
            {children}
          </div>
        </TableFooterSlotProvider>
      </TableToolbarSlotProvider>
      <div
        className={cn(
          'sticky bottom-0 z-10 flex items-center gap-3 h-11 px-3 border-t-[0.5px] border-foreground/[0.102] bg-card-table shrink-0',
          !hasFooter && 'has-[>.footer-slot:empty]:hidden'
        )}
      >
        {/* `w-0 flex-1`: takes the leftover width without letting a long tab strip widen the page. */}
        <div className="flex flex-1 w-0 items-center min-w-0">{footer}</div>
        <div ref={setFooterSlot} className="footer-slot flex items-center shrink-0" />
      </div>
    </div>
  )
}

interface ListToolbarSearchProps {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
  /** Names the input for screen readers when the placeholder isn't enough. */
  'aria-label'?: string
  maxLength?: number
}

// Where the toolbar's search goes: among the actions on the card's right.
const ToolbarSearchSlotContext = createContext<HTMLElement | null>(null)

const isTyping = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))

/**
 * The list's search: an icon among the toolbar's actions on the right, opening into a field.
 * Opens on a click or on "/", focused; Escape clears and closes it; leaving it empty folds it
 * back. While it holds a query it stays open, so the filtering shows.
 */
export const ListToolbarSearch = ({
  value,
  onChange,
  placeholder = 'Search in view...',
  className,
  'aria-label': ariaLabel,
  maxLength,
}: ListToolbarSearchProps) => {
  const slot = useContext(ToolbarSearchSlotContext)
  const inputRef = useRef<HTMLInputElement>(null)
  const [open, setOpen] = useState(value.length > 0)
  const expanded = open || value.length > 0

  const openSearch = () => {
    setOpen(true)
    requestAnimationFrame(() => inputRef.current?.focus())
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || isTyping(event.target)) return
      event.preventDefault()
      openSearch()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  const search = (
    <div
      className={cn(
        // Closed: a 30px icon button. Open: a field (see Input), widening from that button.
        'relative flex h-[30px] shrink-0 items-center overflow-hidden rounded-md border transition-[width,background-color,border-color,box-shadow] duration-200 ease-out motion-reduce:transition-none',
        expanded
          ? 'w-56 border-field-border bg-field focus-within:border-foreground/25 focus-within:ring-[3px] focus-within:ring-foreground/8'
          : 'w-[30px] border-transparent hover:bg-foreground/[0.06]',
        className
      )}
    >
      <button
        type="button"
        onClick={openSearch}
        aria-label="Search"
        tabIndex={expanded ? -1 : 0}
        className="absolute inset-y-0 left-0 flex w-[28px] items-center justify-center text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none"
      >
        <SearchIcon />
      </button>
      <input
        ref={inputRef}
        type="text"
        value={value}
        onChange={e => onChange(e.target.value)}
        onBlur={() => {
          if (value.length === 0) setOpen(false)
        }}
        onKeyDown={e => {
          if (e.key === 'Escape') {
            onChange('')
            setOpen(false)
            e.currentTarget.blur()
          }
        }}
        placeholder={placeholder}
        aria-label={ariaLabel ?? placeholder}
        maxLength={maxLength}
        tabIndex={expanded ? 0 : -1}
        aria-hidden={!expanded}
        className={cn(
          'h-full w-full min-w-0 bg-transparent pl-7 pr-7 text-[13px] text-field-foreground placeholder:text-field-placeholder transition-opacity duration-150 focus:outline-none motion-reduce:transition-none',
          expanded ? 'opacity-100' : 'pointer-events-none opacity-0'
        )}
      />
      {value.length > 0 && (
        <button
          type="button"
          onClick={() => {
            onChange('')
            inputRef.current?.focus()
          }}
          aria-label="Clear search"
          className="absolute right-1.5 flex size-5 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-foreground/[0.08] hover:text-foreground"
        >
          <XIcon size={12} />
        </button>
      )}
    </div>
  )

  return slot ? createPortal(search, slot) : search
}

/** Thin vertical rule between toolbar controls. */
export const ToolbarDivider = () => (
  <span aria-hidden className="h-4 w-px shrink-0 bg-foreground/[0.102]" />
)

interface RefreshButtonProps {
  isLoading?: boolean
  onClick?: () => void
}

/** Ghost refresh button matching the table view-options control. */
export const RefreshButton = ({ isLoading, onClick }: RefreshButtonProps) => {
  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-[30px] w-[30px] text-muted-foreground hover:text-foreground"
      disabled={isLoading}
      onClick={onClick}
      aria-label="Refresh"
    >
      <RefreshCwIcon size={14} className={isLoading ? 'animate-spin' : ''} />
    </Button>
  )
}

interface ListPageLayoutProps extends ListPageHeaderProps {
  /** Card footer, pinned to the bottom — e.g. the saved-view tabs (see {@link TableCard}). */
  footer?: ReactNode
  /** Left side of the card toolbar — search + filters. */
  toolbar?: ReactNode
  /** Right side of the card toolbar — e.g. refresh. */
  toolbarTrailing?: ReactNode
  /** The table (or empty state) rendered inside the card. */
  children: ReactNode
  /** Render children directly without the card surface (e.g. empty states). */
  noCard?: boolean
  className?: string
}

/**
 * The unified core list-page layout: compact header + card-wrapped table.
 */
export const ListPageLayout = ({
  title,
  breadcrumbs,
  tabs,
  actions,
  footer,
  toolbar,
  toolbarTrailing,
  children,
  noCard,
  className,
}: ListPageLayoutProps) => {
  return (
    <div data-full-bleed className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <ListPageHeader title={title} breadcrumbs={breadcrumbs} tabs={tabs} actions={actions} />
      {noCard ? (
        <div className="flex flex-1 flex-col px-3 pb-4">{children}</div>
      ) : (
        <TableCard footer={footer} toolbar={toolbar} toolbarTrailing={toolbarTrailing}>
          {children}
        </TableCard>
      )}
    </div>
  )
}

interface DetailPageLayoutProps {
  /** The section's list, linked: e.g. `{ label: 'Invoices', to: '…/invoices' }`. */
  section: Crumb
  /** The item itself: its number or name. */
  title: ReactNode
  /** Right of the bar, as on the list pages. */
  actions?: ReactNode
  children: ReactNode
  /** Classes for the content under the bar (24px sides by default). */
  className?: string
}

/**
 * Every detail page's frame: the list pages' top bar (the section's icon, then the section and
 * the item as a breadcrumb, actions on the right), then the page's content. Runs edge to edge
 * like the list pages, and gives the content its own 24px sides.
 */
export const DetailPageLayout = ({
  section,
  title,
  actions,
  children,
  className,
}: DetailPageLayoutProps) => (
  <div data-full-bleed className="flex min-h-0 flex-1 flex-col">
    <ListPageHeader title={title} breadcrumbs={[section]} actions={actions} />
    <div className={cn('flex min-h-0 flex-1 flex-col px-6 pb-6', className)}>{children}</div>
  </div>
)
