import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  cn,
} from '@md/ui'
import { XIcon } from 'lucide-react'
import { forwardRef, useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { useSearchParams } from 'react-router'

import type { LucideIcon } from 'lucide-react'

export interface PlanExtra {
  id: string
  title: string
  /** What it is, under the title once opened. */
  subtitle: string
  icon: LucideIcon
  /** Set up: shown as a summary card, else as a dashed "Add" card. */
  configured: boolean
  /** What is set, read on the card: "14 days", "2 offered · Priority support". */
  summary?: string
  /** What adding it does, read on a dashed card. */
  pitch: string
  /** A small form opens in a popover, a list in a panel under the cards, a long form in a sheet. */
  opens: 'popover' | 'panel' | 'sheet'
  /** Not set up yet: the card starts it right away, nothing to open first. */
  onAdd?: () => void
  /** Shown, not edited here (a published version's): the card reads "View". */
  readOnly?: boolean
  content: ReactNode | ((close: () => void) => ReactNode)
}

/** The plan's other sections as cards; each opens where its content fits. */
export const MoreForPlan = ({ extras }: { extras: PlanExtra[] }) => {
  // `?more=coupons` opens that section, as links from elsewhere do.
  const [params] = useSearchParams()
  const [openId, setOpenId] = useState<string | undefined>(() => params.get('more') ?? undefined)
  const titleId = useId()
  const panelRef = useRef<HTMLDivElement>(null)
  const open = extras.find(extra => extra.id === openId)
  const close = () => setOpenId(undefined)

  useEffect(() => {
    if (open?.opens === 'panel') panelRef.current?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  }, [open?.id])

  if (extras.length === 0) return null

  const body = (extra: PlanExtra) =>
    typeof extra.content === 'function' ? extra.content(close) : extra.content

  return (
    <section aria-labelledby={titleId} className="space-y-3 pb-6">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 id={titleId} className="text-lg font-semibold">
          More for this plan
        </h2>
        <span className="text-sm text-muted-foreground">Optional, each set up on its own</span>
      </div>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {extras.map(extra => {
          const isOpen = extra.id === openId
          const card = (
            <ExtraCard
              extra={extra}
              active={isOpen}
              onClick={
                extra.opens === 'popover'
                  ? undefined
                  : () =>
                      !extra.configured && extra.onAdd
                        ? extra.onAdd()
                        : setOpenId(isOpen ? undefined : extra.id)
              }
            />
          )
          if (extra.opens !== 'popover') return <div key={extra.id}>{card}</div>
          return (
            <Popover
              key={extra.id}
              open={isOpen}
              onOpenChange={next => setOpenId(next ? extra.id : undefined)}
            >
              <PopoverTrigger asChild>{card}</PopoverTrigger>
              <PopoverContent
                align="start"
                collisionPadding={16}
                className="flex max-h-[var(--radix-popover-content-available-height)] w-[30rem] max-w-[calc(100vw-2rem)] flex-col p-0"
              >
                <ExtraHeader extra={extra} />
                <div className="overflow-y-auto p-4">{isOpen && body(extra)}</div>
              </PopoverContent>
            </Popover>
          )
        })}
      </div>

      {open?.opens === 'panel' && (
        <div
          ref={panelRef}
          role="region"
          aria-label={open.title}
          className="scroll-mt-4 overflow-hidden rounded-lg border border-border bg-card"
        >
          <ExtraHeader extra={open} onClose={close} />
          <div className="p-4">{body(open)}</div>
        </div>
      )}

      <Sheet open={open?.opens === 'sheet'} onOpenChange={next => !next && close()}>
        <SheetContent size="medium" className="flex flex-col gap-0 p-0">
          {open?.opens === 'sheet' && (
            <>
              <SheetHeader className="border-b border-border px-6 py-4 pr-12">
                <SheetTitle className="text-base">{open.title}</SheetTitle>
                <SheetDescription className="text-xs">{open.subtitle}</SheetDescription>
              </SheetHeader>
              <div className="flex min-h-0 flex-1 flex-col px-6 pt-4">{body(open)}</div>
            </>
          )}
        </SheetContent>
      </Sheet>
    </section>
  )
}

const ExtraIcon = ({ icon: Icon }: { icon: LucideIcon }) => (
  <span className="inline-flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
    <Icon size={14} aria-hidden />
  </span>
)

const ExtraCard = forwardRef<
  HTMLButtonElement,
  { extra: PlanExtra; active: boolean; onClick?: () => void }
>(({ extra, active, onClick, ...props }, ref) => (
  <button
    ref={ref}
    type="button"
    onClick={onClick}
    aria-expanded={extra.opens === 'panel' ? active : undefined}
    {...props}
    className={cn(
      'flex h-full w-full items-center gap-3 rounded-lg px-3.5 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
      extra.configured
        ? 'border border-border bg-card hover:bg-accent/50'
        : 'border border-dashed border-border hover:border-muted-foreground/40 hover:bg-accent/30',
      active && 'border-solid border-muted-foreground/50 bg-accent/50'
    )}
  >
    <ExtraIcon icon={extra.icon} />
    <span className="min-w-0 flex-1">
      <span className="block text-sm font-medium leading-snug text-foreground">{extra.title}</span>
      <span className="line-clamp-2 text-xs text-muted-foreground">
        {extra.configured ? extra.summary || 'Set up' : extra.pitch}
      </span>
    </span>
    <span className="shrink-0 text-xs font-medium text-muted-foreground">
      {!extra.configured ? 'Add' : extra.readOnly ? 'View' : 'Edit'}
    </span>
  </button>
))
ExtraCard.displayName = 'ExtraCard'

const ExtraHeader = ({ extra, onClose }: { extra: PlanExtra; onClose?: () => void }) => (
  <header className="flex items-start gap-3 border-b border-border px-4 py-3">
    <ExtraIcon icon={extra.icon} />
    <div className="min-w-0 flex-1">
      <h3 className="text-sm font-semibold text-foreground">{extra.title}</h3>
      <p className="text-xs text-muted-foreground">{extra.subtitle}</p>
    </div>
    {onClose && (
      <Button
        variant="ghost"
        size="icon"
        className="size-7 shrink-0 text-muted-foreground"
        aria-label={`Close ${extra.title}`}
        onClick={onClose}
      >
        <XIcon size={14} />
      </Button>
    )}
  </header>
)

/** The buttons closing a section's form: `start` on the left (e.g. a removal), the rest right. */
export const ExtraActions = ({ start, children }: { start?: ReactNode; children: ReactNode }) => (
  <div className="mt-4 flex items-center gap-2 border-t border-border pt-3">
    {start}
    <div className="ml-auto flex items-center gap-2">{children}</div>
  </div>
)

/** The dashed "+ Add …" control ending a section's list, as the plan's other lists do. */
export const AddRow = forwardRef<HTMLButtonElement, React.ComponentProps<'button'>>(
  ({ className, children, ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      {...props}
      className={cn(
        'inline-flex h-8 items-center gap-1.5 rounded-md border border-dashed border-border px-3 text-xs text-muted-foreground transition-colors hover:border-muted-foreground/40 hover:text-foreground disabled:pointer-events-none disabled:opacity-50',
        className
      )}
    >
      {children}
    </button>
  )
)
AddRow.displayName = 'AddRow'
