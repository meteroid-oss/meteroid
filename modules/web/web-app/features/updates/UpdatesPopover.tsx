import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Popover,
  PopoverContent,
  PopoverTrigger,
  SidebarMenuButton,
  SidebarMenuItem,
  cn,
} from '@md/ui'
import { format } from 'date-fns'
import { BellIcon } from 'lucide-react'
import { useState } from 'react'

import { useUpdates } from './useUpdates'

import type { ChangelogBadge, ChangelogEntry } from './changelog'
import type { FunctionComponent } from 'react'

// The bell surfaces only the most recent updates; there's no "load more".
const MAX_VISIBLE_UPDATES = 5

const BADGE_STYLES: Record<ChangelogBadge, { label: string; className: string }> = {
  new: { label: 'New', className: 'bg-brand/10 text-brand' },
  improvement: { label: 'Improved', className: 'bg-success/10 text-success' },
  fix: { label: 'Fix', className: 'bg-muted text-muted-foreground' },
}

const UpdatesPopover: FunctionComponent = () => {
  const { entries, hasUnread, isUnread, markAllSeen } = useUpdates()
  const [selected, setSelected] = useState<ChangelogEntry | null>(null)

  const onOpenChange = (open: boolean) => {
    if (open && hasUnread) markAllSeen()
  }

  return (
    <SidebarMenuItem>
      <Popover onOpenChange={onOpenChange}>
        <PopoverTrigger asChild>
          <SidebarMenuButton size="sm" className="relative">
            <BellIcon />
            <span>What&apos;s new</span>
            {hasUnread && (
              <span className="absolute left-[18px] top-1 h-2 w-2 rounded-full bg-brand ring-2 ring-sidebar" />
            )}
          </SidebarMenuButton>
        </PopoverTrigger>

        <PopoverContent side="right" align="end" sideOffset={12} className="w-[360px] p-0">
          <div className="flex items-center px-4 py-3 border-b">
            <h3 className="text-sm font-semibold">What&apos;s new</h3>
          </div>

          {entries.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-muted-foreground">
              You&apos;re all caught up.
            </div>
          ) : (
            <div className="max-h-[500px] overflow-y-auto">
              <ul className="divide-y">
                {entries.slice(0, MAX_VISIBLE_UPDATES).map(entry => (
                  <UpdateRow
                    key={entry.id}
                    entry={entry}
                    unread={isUnread(entry.date)}
                    onOpen={() => setSelected(entry)}
                  />
                ))}
              </ul>
            </div>
          )}
        </PopoverContent>
      </Popover>

      <UpdateDialog entry={selected} onClose={() => setSelected(null)} />
    </SidebarMenuItem>
  )
}

const Badge: FunctionComponent<{ badge?: ChangelogBadge }> = ({ badge }) => {
  if (!badge) return null
  const { label, className } = BADGE_STYLES[badge]
  return (
    <span
      className={cn(
        'inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium leading-none',
        className
      )}
    >
      {label}
    </span>
  )
}

const UpdateRow: FunctionComponent<{
  entry: ChangelogEntry
  unread: boolean
  onOpen: () => void
}> = ({ entry, unread, onOpen }) => (
  <li>
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        'w-full text-left px-4 py-3.5 transition-colors hover:bg-accent/60',
        unread && 'bg-accent/40'
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        <Badge badge={entry.badge} />
        <span className="text-[11px] text-muted-foreground">
          {format(new Date(entry.date), 'MMM d, yyyy')}
        </span>
        {unread && <span className="ml-auto h-1.5 w-1.5 rounded-full bg-brand" />}
      </div>
      <h4 className="text-sm font-medium leading-snug">{entry.title}</h4>
      {entry.summary && (
        <p className="mt-0.5 text-xs text-muted-foreground leading-relaxed">{entry.summary}</p>
      )}
      {entry.cover && (
        <img
          src={entry.cover}
          alt=""
          loading="lazy"
          className="mt-2.5 w-full rounded-md border object-cover"
        />
      )}
    </button>
  </li>
)

const UpdateDialog: FunctionComponent<{ entry: ChangelogEntry | null; onClose: () => void }> = ({
  entry,
  onClose,
}) => {
  return (
    <Dialog open={!!entry} onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-lg p-0 overflow-hidden">
        {entry && (
          <>
            <DialogHeader className="px-6 pt-6 pb-3 space-y-2">
              <div className="flex items-center gap-2">
                <Badge badge={entry.badge} />
                <span className="text-xs text-muted-foreground">
                  {format(new Date(entry.date), 'MMMM d, yyyy')}
                </span>
              </div>
              <DialogTitle className="text-lg">{entry.title}</DialogTitle>
            </DialogHeader>
            <div className="max-h-[70vh] overflow-y-auto">
              <UpdateBody entry={entry} />
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

// Renders the compiled MDX body. Styling targets the raw markdown elements.
const UpdateBody: FunctionComponent<{ entry: ChangelogEntry }> = ({ entry }) => {
  const { Body } = entry

  return (
    <div
      className={cn(
        'px-6 pb-6 text-sm text-muted-foreground leading-relaxed',
        '[&_h2]:mt-5 [&_h2]:mb-1.5 [&_h2]:text-sm [&_h2]:font-semibold [&_h2]:text-foreground',
        '[&_p]:my-2 [&_a]:text-brand [&_a]:underline',
        '[&_img]:my-3 [&_img]:rounded-md [&_img]:border [&_img]:w-full',
        '[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5 [&_li]:mt-1'
      )}
    >
      <Body />
    </div>
  )
}

export default UpdatesPopover
