import { cn } from '@md/ui'
import { ReactNode } from 'react'

// Compact, consistent settings layout primitives shared across every settings tab.
// Section = titled block separated by a hairline; Row = label/description left,
// control right; FieldGrid = responsive two-column form grid.

export const SettingsSection = ({
  id,
  title,
  description,
  action,
  children,
  className,
  divided = true,
}: {
  /** An anchor an in-page nav scrolls to. */
  id?: string
  /** Left out when the page's own title already says it (API keys, Webhooks…). */
  title?: ReactNode
  description?: ReactNode
  action?: ReactNode
  children?: ReactNode
  className?: string
  // Draws the hairline separator above the section. Set false on the first
  // section of a tab. A prop (not a CSS `:first-child`) so it also works when
  // consecutive sections live in separate <form> wrappers.
  divided?: boolean
}) => (
  <section
    id={id}
    className={cn('scroll-mt-16', divided ? 'border-t border-border pt-6' : 'pt-0', className)}
  >
    <div className="mb-4 flex items-start justify-between gap-3">
      <div className="space-y-0.5">
        {title && <h3 className="text-sm font-semibold tracking-tight">{title}</h3>}
        {description && (
          <p className="max-w-xl text-[13px] leading-snug text-muted-foreground">{description}</p>
        )}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
    {children}
  </section>
)

export const SettingsRowGroup = ({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) => (
  <div className={cn('overflow-hidden rounded-lg border border-border bg-card', className)}>
    {children}
  </div>
)

/**
 * The tile leading a row's title (an integration, a payment method): white at 7.2% in dark, its
 * icon a step quieter than the title. A logo with its own colours keeps them.
 */
export const SettingsRowIcon = ({ children }: { children: ReactNode }) => (
  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground dark:bg-white/[0.072] [&_svg]:h-4 [&_svg]:w-4">
    {children}
  </span>
)

export const SettingsRow = ({
  title,
  description,
  children,
  className,
}: {
  title: ReactNode
  description?: ReactNode
  children?: ReactNode
  className?: string
}) => (
  <div
    className={cn(
      'flex items-center gap-4 border-t border-border px-4 py-3 first:border-t-0',
      className
    )}
  >
    <div className="min-w-0 space-y-0.5">
      <div className="flex items-center gap-2 text-[13px] font-medium">{title}</div>
      {description && (
        <div className="text-xs leading-snug text-muted-foreground">{description}</div>
      )}
    </div>
    {children != null && (
      <div className="ml-auto flex shrink-0 items-center gap-2.5">{children}</div>
    )}
  </div>
)

// The empty state is shared with the detail pages; Settings keeps its name for it.
export { SectionEmptyState as SettingsEmptyState } from '@/components/empty-state/SectionEmptyState'

export const SettingsFieldGrid = ({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) => (
  <div className={cn('grid grid-cols-1 gap-x-4 gap-y-3.5 sm:grid-cols-2', className)}>
    {children}
  </div>
)
