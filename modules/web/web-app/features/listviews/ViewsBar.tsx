import { ConnectError } from '@connectrpc/connect'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Label,
  Switch,
} from '@md/ui'
import { cn } from '@ui/lib'
import {
  CheckIcon,
  ChevronDownIcon,
  LockIcon,
  MoreHorizontalIcon,
  PlusIcon,
  StarIcon,
  Table2Icon,
  UsersIcon,
} from 'lucide-react'
import { ReactNode, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'

import { EntityListView } from '@/rpc/api/listviews/v1/models_pb'

import { SavedViews } from './useSavedViews'

const NAME_MAX = 100

const errorMessage = (e: unknown, fallback: string) =>
  e instanceof ConnectError ? e.rawMessage : e instanceof Error ? e.message : fallback

const notify = <T,>(p: Promise<T>, success: string, failure: string) =>
  p.then(
    v => {
      toast.success(success)
      return v
    },
    e => {
      toast.error(errorMessage(e, failure))
      throw e
    }
  )

type NameDialogState = { mode: 'create' } | { mode: 'rename'; view: EntityListView } | undefined

interface Props {
  views: SavedViews
  /** Plural noun for the unfiltered tab, e.g. "customers". */
  entityLabel: string
}

/**
 * Saved-view tabs for a list's card footer (sheet-tab style): "All" plus one tab per view the user
 * can see, per-view actions, and save/revert when the current state drifts from the active tab.
 * "All" is a fixed view (no filters, default sort): it shows drift like any other tab, but can
 * only be saved as a new view.
 */
export const ViewsBar = ({ views: sv, entityLabel }: Props) => {
  const [nameDialog, setNameDialog] = useState<NameDialogState>()
  const [toDelete, setToDelete] = useState<EntityListView>()
  const { activeView } = sv
  const modified = activeView ? sv.isDirty : sv.isCustomized
  const allLabel = `All ${entityLabel}`

  // Keep the active tab in view, and fade the strip's edge once the tabs overflow.
  const tabsRef = useRef<HTMLDivElement>(null)
  const [overflowing, setOverflowing] = useState(false)
  useEffect(() => {
    const el = tabsRef.current
    if (!el) return
    // No initial call: `observe` reports the first size after layout, so mounting never forces one.
    const measure = () => setOverflowing(el.scrollWidth > el.clientWidth + 1)
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [sv.views.length])
  // Horizontal only: `scrollIntoView` would also scroll the page to the footer's layout position.
  useEffect(() => {
    const list = tabsRef.current
    const tab = list?.querySelector('[aria-selected="true"]')?.parentElement
    if (!list || !tab) return
    const l = list.getBoundingClientRect()
    const t = tab.getBoundingClientRect()
    if (t.left < l.left) list.scrollLeft -= l.left - t.left
    else if (t.right > l.right) list.scrollLeft += t.right - l.right
  }, [activeView?.id])

  const save = () =>
    notify(sv.saveActive(), 'View saved', 'Failed to save view').catch(() => undefined)

  return (
    <div className="flex items-center gap-1 min-w-0 w-full">
      <div
        ref={tabsRef}
        role="tablist"
        className={cn(
          'flex items-center gap-0.5 min-w-0 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
          // Once tabs no longer fit, fade the clipped edge so it reads as "scrolls", not "cut off".
          overflowing && '[mask-image:linear-gradient(to_right,black_calc(100%-2rem),transparent)]'
        )}
      >
        <ViewTab
          label={allLabel}
          active={!activeView}
          modified={!activeView && modified}
          onSelect={() => sv.selectView(null)}
        />
        {sv.views.map(view =>
          sv.isPreset(view) ? (
            <PresetViewTab
              key={view.id}
              view={view}
              active={activeView?.id === view.id}
              modified={activeView?.id === view.id && modified}
              isDefault={sv.defaultViewId === view.id}
              onSelect={() => sv.selectView(view)}
              onToggleDefault={() =>
                notify(
                  sv.setDefault(sv.defaultViewId === view.id ? null : view),
                  sv.defaultViewId === view.id
                    ? 'Default view removed'
                    : `“${view.name}” will open by default`,
                  'Failed to update default view'
                ).catch(() => undefined)
              }
            />
          ) : (
            <SavedViewTab
              key={view.id}
              view={view}
              active={activeView?.id === view.id}
              modified={activeView?.id === view.id && modified}
              isDefault={sv.defaultViewId === view.id}
              onSelect={() => sv.selectView(view)}
              onRename={() => setNameDialog({ mode: 'rename', view })}
              onDuplicate={() =>
                notify(sv.duplicate(view), 'View duplicated', 'Failed to duplicate view').catch(
                  () => undefined
                )
              }
              onToggleShared={() =>
                notify(
                  sv.setShared(view, !view.shared),
                  view.shared ? 'View is now private' : 'View shared with your workspace',
                  'Failed to update sharing'
                ).catch(() => undefined)
              }
              onToggleDefault={() =>
                notify(
                  sv.setDefault(sv.defaultViewId === view.id ? null : view),
                  sv.defaultViewId === view.id
                    ? 'Default view removed'
                    : `“${view.name}” will open by default`,
                  'Failed to update default view'
                ).catch(() => undefined)
              }
              onDelete={() => setToDelete(view)}
            />
          )
        )}
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0 mt-0 shrink-0 text-muted-foreground"
            aria-label="All views"
            title="All views"
          >
            <ChevronDownIcon size={14} />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" side="top" className="w-64 max-h-96 overflow-y-auto">
          {[null, ...sv.views].map(view => {
            const selected = (view?.id ?? null) === (activeView?.id ?? null)
            return (
              <DropdownMenuItem
                key={view?.id ?? 'all'}
                className="gap-2 text-xs"
                onSelect={() => !selected && sv.selectView(view)}
              >
                <CheckIcon size={12} className={cn('shrink-0', !selected && 'opacity-0')} />
                <span className="truncate">{view?.name ?? allLabel}</span>
                <span className="ml-auto flex items-center gap-1.5 shrink-0 text-muted-foreground">
                  {view?.shared && <UsersIcon size={11} aria-label="Shared" />}
                  {view && sv.defaultViewId === view.id && (
                    <StarIcon size={11} aria-label="Your default" />
                  )}
                  {selected && modified && (
                    <span
                      className="h-1.5 w-1.5 rounded-full bg-brand"
                      aria-label="Unsaved changes"
                    />
                  )}
                </span>
              </DropdownMenuItem>
            )
          })}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="gap-2 text-xs"
            onSelect={() => setNameDialog({ mode: 'create' })}
          >
            <PlusIcon size={12} className="shrink-0" />
            New view from current state…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Button
        variant="ghost"
        size="sm"
        className="h-7 w-7 p-0 mt-0 shrink-0 text-muted-foreground"
        aria-label="New view"
        title="Save current filters, sort and columns as a new view"
        onClick={() => setNameDialog({ mode: 'create' })}
      >
        <PlusIcon size={14} />
      </Button>

      {modified && (
        <div className="flex items-center gap-1 shrink-0 pl-2 ml-1 border-l border-border">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 mt-0 text-xs"
            onClick={() => (activeView ? sv.revert() : sv.selectView(null))}
          >
            Revert
          </Button>
          <Button
            variant={activeView ? 'ghost' : 'secondary'}
            size="sm"
            className="h-7 mt-0 text-xs"
            onClick={() => setNameDialog({ mode: 'create' })}
          >
            {activeView ? 'Save as new' : 'Save as view'}
          </Button>
          {activeView && !sv.isPreset(activeView) && (
            <Button
              size="sm"
              className="h-7 mt-0 text-xs"
              disabled={sv.isSaving}
              title={
                activeView.shared ? 'Updates this view for everyone in the workspace' : undefined
              }
              onClick={save}
            >
              {activeView.shared ? 'Save for everyone' : 'Save view'}
            </Button>
          )}
        </div>
      )}

      <ViewNameDialog
        state={nameDialog}
        pending={sv.isSaving}
        onClose={() => setNameDialog(undefined)}
        onSubmit={async (name, shared) => {
          if (!nameDialog) return
          const p =
            nameDialog.mode === 'create'
              ? notify(sv.createFromCurrent(name, shared), 'View created', 'Failed to create view')
              : notify(sv.rename(nameDialog.view, name), 'View renamed', 'Failed to rename view')
          await p.then(() => setNameDialog(undefined)).catch(() => undefined)
        }}
      />

      <AlertDialog open={!!toDelete} onOpenChange={open => !open && setToDelete(undefined)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete “{toDelete?.name}”?</AlertDialogTitle>
            <AlertDialogDescription>
              {toDelete?.shared
                ? 'This view is shared with your workspace. Deleting it removes it for everyone.'
                : 'This removes the view. The records it shows are not affected.'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (!toDelete) return
                notify(sv.remove(toDelete), 'View deleted', 'Failed to delete view').catch(
                  () => undefined
                )
                setToDelete(undefined)
              }}
            >
              Delete view
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

/** One tab. Clicking the active tab is a no-op, so it can never discard unsaved changes. */
const ViewTab = ({
  label,
  active,
  shared,
  isDefault,
  modified,
  onSelect,
  onDoubleClick,
  onContextMenu,
  menu,
}: {
  label: string
  active: boolean
  shared?: boolean
  isDefault?: boolean
  modified?: boolean
  onSelect: () => void
  onDoubleClick?: () => void
  onContextMenu?: (e: React.MouseEvent) => void
  menu?: ReactNode
}) => (
  <div
    className={cn(
      // Chrome-style: inactive tabs shrink (names truncate) down to a minimum width before the strip
      // starts scrolling; the active tab keeps its full name.
      'group relative flex items-center h-7 rounded-md transition-colors max-w-[14rem]',
      active ? 'shrink-0' : 'shrink min-w-[5.5rem]',
      active ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted/60'
    )}
    onContextMenu={onContextMenu}
  >
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={() => !active && onSelect()}
      onDoubleClick={onDoubleClick}
      className={cn(
        'flex items-center gap-1.5 h-full min-w-0 text-xs font-medium',
        menu && active ? 'pl-2 pr-1' : 'px-2'
      )}
    >
      <Table2Icon size={13} className="shrink-0 opacity-70" />
      <span className="min-w-0 truncate" title={label}>
        {label}
      </span>
      {shared && <UsersIcon size={11} className="shrink-0 opacity-60" aria-label="Shared" />}
      {isDefault && (
        <StarIcon size={11} className="shrink-0 opacity-60" aria-label="Your default" />
      )}
      {modified && (
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full bg-brand"
          role="status"
          aria-label="Unsaved changes"
          title="Unsaved changes — save or revert them next to the tabs"
        />
      )}
    </button>
    {menu && (
      <div
        className={cn(
          'shrink-0',
          // Inactive tabs don't reserve room for ⋯: it floats over the name's end on hover.
          active ? 'pr-0.5' : 'absolute right-0.5 rounded bg-muted invisible group-hover:visible'
        )}
      >
        {menu}
      </div>
    )}
  </div>
)

/** A built-in view's tab: it can only be pinned as one's default. */
const PresetViewTab = ({
  view,
  active,
  modified,
  isDefault,
  onSelect,
  onToggleDefault,
}: {
  view: EntityListView
  active: boolean
  modified: boolean
  isDefault: boolean
  onSelect: () => void
  onToggleDefault: () => void
}) => {
  const [menuOpen, setMenuOpen] = useState(false)

  return (
    <ViewTab
      label={view.name}
      active={active}
      isDefault={isDefault}
      modified={modified}
      onSelect={onSelect}
      onContextMenu={e => {
        e.preventDefault()
        setMenuOpen(true)
      }}
      menu={
        <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className={cn(
                'flex h-5 w-5 items-center justify-center rounded hover:bg-background/80',
                menuOpen && 'visible'
              )}
              aria-label={`Options for ${view.name}`}
            >
              <MoreHorizontalIcon size={12} />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-56">
            <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
              Built-in view
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onToggleDefault}>
              {isDefault ? 'Remove as my default' : 'Set as my default'}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
    />
  )
}

/** A saved view's tab: double-click renames, right-click (or ⋯) opens its menu. */
const SavedViewTab = ({
  view,
  active,
  modified,
  isDefault,
  onSelect,
  onRename,
  onDuplicate,
  onToggleShared,
  onToggleDefault,
  onDelete,
}: {
  view: EntityListView
  active: boolean
  modified: boolean
  isDefault: boolean
  onSelect: () => void
  onRename: () => void
  onDuplicate: () => void
  onToggleShared: () => void
  onToggleDefault: () => void
  onDelete: () => void
}) => {
  const [menuOpen, setMenuOpen] = useState(false)

  return (
    <ViewTab
      label={view.name}
      active={active}
      shared={view.shared}
      isDefault={isDefault}
      modified={modified}
      onSelect={onSelect}
      onDoubleClick={onRename}
      onContextMenu={e => {
        e.preventDefault()
        setMenuOpen(true)
      }}
      menu={
        <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className={cn(
                'flex h-5 w-5 items-center justify-center rounded hover:bg-background/80',
                // Visible while open, even on an inactive (hover-only) tab.
                menuOpen && 'visible'
              )}
              aria-label={`Options for ${view.name}`}
            >
              <MoreHorizontalIcon size={12} />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-56">
            <DropdownMenuLabel className="flex items-center gap-1.5 text-xs font-normal text-muted-foreground">
              {view.shared ? (
                <>
                  <UsersIcon size={12} />
                  Shared
                  {view.isOwner ? ' by you' : view.createdByName ? ` by ${view.createdByName}` : ''}
                </>
              ) : (
                <>
                  <LockIcon size={12} />
                  Private · only you can see it
                </>
              )}
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onToggleDefault}>
              {isDefault ? 'Remove as my default' : 'Set as my default'}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onRename}>Rename…</DropdownMenuItem>
            <DropdownMenuItem onSelect={onDuplicate}>Duplicate as private view</DropdownMenuItem>
            {view.isOwner && (
              <DropdownMenuItem onSelect={onToggleShared}>
                {view.shared ? 'Make private' : 'Share with workspace'}
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              onSelect={onDelete}
            >
              Delete…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
    />
  )
}

const ViewNameDialog = ({
  state,
  pending,
  onClose,
  onSubmit,
}: {
  state: NameDialogState
  pending: boolean
  onClose: () => void
  onSubmit: (name: string, shared: boolean) => void
}) => {
  const [name, setName] = useState('')
  const [shared, setShared] = useState(false)
  const [lastState, setLastState] = useState<NameDialogState>()

  // Reset the form whenever the dialog opens for a new purpose.
  if (state !== lastState) {
    setLastState(state)
    setName(state?.mode === 'rename' ? state.view.name : '')
    setShared(false)
  }

  const trimmed = name.trim()
  const valid = trimmed.length > 0 && trimmed.length <= NAME_MAX
  const submit = () => valid && !pending && onSubmit(trimmed, shared)

  return (
    <Dialog open={!!state} onOpenChange={open => !open && onClose()}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>{state?.mode === 'rename' ? 'Rename view' : 'New view'}</DialogTitle>
          {state?.mode === 'create' && (
            <DialogDescription>
              Saves the current filters, sort and visible columns.
            </DialogDescription>
          )}
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <Input
            autoFocus
            placeholder="View name"
            value={name}
            maxLength={NAME_MAX}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && submit()}
          />
          {state?.mode === 'create' && (
            <div className="flex items-start justify-between gap-4">
              <div className="flex flex-col gap-0.5">
                <Label htmlFor="view-shared" className="text-sm">
                  Share with workspace
                </Label>
                <span className="text-xs text-muted-foreground">
                  Everyone in this workspace can use and edit it. Otherwise only you see it.
                </span>
              </div>
              <Switch id="view-shared" checked={shared} onCheckedChange={setShared} />
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" disabled={!valid || pending} onClick={submit}>
            {state?.mode === 'rename' ? 'Rename' : 'Create view'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
