import { SearchIcon } from '@md/icons'
import { Button, cn } from '@md/ui'
import { ChevronRight, Home, RefreshCwIcon } from 'lucide-react'
import { Fragment, ReactNode, useState } from 'react'
import { Link } from 'react-router-dom'

import {
  TableFooterSlotProvider,
  TableToolbarSlotProvider,
} from '@/components/table/TableToolbarSlot'
import { useBasePath } from '@/hooks/useBasePath'

export interface Crumb {
  label: ReactNode
  to?: string
}

interface ListPageHeaderProps {
  title: ReactNode
  count?: number
  breadcrumbs?: Crumb[]
  tabs?: ReactNode
  actions?: ReactNode
}

/**
 * Compact page header shared by every list page: a home-rooted breadcrumb on
 * the left, optional centered tabs, and optional actions on the right.
 */
export const ListPageHeader = ({
  title,
  count,
  breadcrumbs,
  tabs,
  actions,
}: ListPageHeaderProps) => {
  const basePath = useBasePath()

  return (
    <div className="flex items-center gap-3 min-h-8">
      <div className="flex flex-1 items-center gap-1.5 min-w-0 text-sm text-muted-foreground">
        <Link
          to={basePath}
          className="shrink-0 rounded-md p-1 hover:bg-accent hover:text-foreground transition-colors"
          aria-label="Home"
        >
          <Home size={15} />
        </Link>
        {breadcrumbs?.map((crumb, i) => (
          <Fragment key={i}>
            <ChevronRight size={14} className="shrink-0 opacity-60" />
            {crumb.to ? (
              <Link to={crumb.to} className="truncate hover:text-foreground transition-colors">
                {crumb.label}
              </Link>
            ) : (
              <span className="truncate">{crumb.label}</span>
            )}
          </Fragment>
        ))}
        <ChevronRight size={14} className="shrink-0 opacity-60" />
        <span className="flex items-center gap-1.5 font-medium text-foreground truncate">
          <span className="truncate">{title}</span>
          {count !== undefined && count >= 0 && (
            <span className="text-xs font-normal text-muted-foreground">{count}</span>
          )}
        </span>
      </div>
      {tabs && <div className="shrink-0">{tabs}</div>}
      <div className="flex flex-1 items-center justify-end gap-2">{actions}</div>
    </div>
  )
}

// Whether a card with a footer stretches to the bottom of the window (footer on the window edge
// even for short lists), or hugs its rows (footer right under the last row). Either way the footer
// sticks to the window bottom while a long list scrolls.
const FOOTER_FILLS_HEIGHT = false

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
 * consistent bordered, card-like surface.
 */
export const TableCard = ({
  footer,
  toolbar,
  toolbarTrailing,
  children,
  className,
}: TableCardProps) => {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null)
  const [footerSlot, setFooterSlot] = useState<HTMLDivElement | null>(null)
  const hasToolbar = toolbar !== undefined || toolbarTrailing !== undefined
  const hasFooter = footer !== undefined

  return (
    // `overflow-clip` rather than `overflow-hidden`: it rounds the corners without making the card
    // a scroll container, which would stop the footer from sticking to the viewport bottom.
    <div
      className={cn(
        'flex flex-col rounded-md border bg-card overflow-clip',
        hasFooter && FOOTER_FILLS_HEIGHT && 'flex-1',
        className
      )}
    >
      {hasToolbar && (
        <div className="flex items-center justify-between gap-2 px-3 min-h-11 py-1.5 border-b border-border shrink-0">
          <div className="flex flex-1 flex-wrap items-center gap-2 min-w-0">{toolbar}</div>
          <div className="flex items-center gap-1 shrink-0">
            {toolbarTrailing}
            <div ref={setSlot} className="flex items-center" />
          </div>
        </div>
      )}
      <TableToolbarSlotProvider value={slot}>
        <TableFooterSlotProvider value={footerSlot}>
          <div
            className={cn(
              'flex flex-col min-w-0 px-1 pb-1',
              hasFooter && FOOTER_FILLS_HEIGHT && 'flex-1'
            )}
          >
            {children}
          </div>
        </TableFooterSlotProvider>
      </TableToolbarSlotProvider>
      <div
        className={cn(
          'sticky bottom-0 z-10 flex items-center gap-3 h-10 px-2 border-t border-border bg-card shrink-0',
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

/** Borderless search input designed to sit inside a TableCard toolbar. */
export const ListToolbarSearch = ({
  value,
  onChange,
  placeholder = 'Search in view...',
  className,
  'aria-label': ariaLabel,
  maxLength,
}: ListToolbarSearchProps) => {
  return (
    <div className={cn('flex items-center gap-2 min-w-0 border-r-1 mr-1', className)}>
      <SearchIcon size={15} className="shrink-0 text-muted-foreground" />
      <input
        type="text"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={ariaLabel}
        maxLength={maxLength}
        className="h-8 w-full min-w-0 bg-transparent text-xs text-foreground placeholder:text-muted-foreground focus:outline-none"
      />
    </div>
  )
}

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
      className="h-8 w-8 text-muted-foreground hover:text-foreground"
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
  count,
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
    <div
      className={cn(
        'flex flex-col gap-4',
        FOOTER_FILLS_HEIGHT && footer !== undefined && !noCard && 'flex-1',
        className
      )}
    >
      <ListPageHeader
        title={title}
        count={count}
        breadcrumbs={breadcrumbs}
        tabs={tabs}
        actions={actions}
      />
      {noCard ? (
        children
      ) : (
        <TableCard footer={footer} toolbar={toolbar} toolbarTrailing={toolbarTrailing}>
          {children}
        </TableCard>
      )}
    </div>
  )
}
