import { cn } from '@md/ui'
import { ReactNode } from 'react'

/**
 * What a section, list or table shows while it is empty (in Settings and on the detail pages),
 * centred in a frame like the table's: one quiet line, or, with an icon and a title, what the
 * list is for and a (secondary) action to add the first. Inside a frame that is already drawn (a row group, a table's
 * container), pass `framed={false}`.
 */
export const SectionEmptyState = ({
  children,
  icon,
  title,
  action,
  framed = true,
  className,
}: {
  children: ReactNode
  icon?: ReactNode
  title?: ReactNode
  action?: ReactNode
  framed?: boolean
  /** E.g. `bg-card` on a detail page, so a lone empty state sits in the same box as a table. */
  className?: string
}) => (
  <div
    className={cn(
      'text-center text-sm text-muted-foreground',
      icon || title || action ? 'flex flex-col items-center gap-3 px-6 py-12' : 'p-8',
      framed && 'rounded-lg border border-border',
      className
    )}
  >
    {icon && <span className="text-muted-foreground [&_svg]:size-5">{icon}</span>}
    {title ? (
      <div className="space-y-1">
        <p className="font-medium text-foreground">{title}</p>
        <p className="max-w-sm">{children}</p>
      </div>
    ) : (
      children
    )}
    {action}
  </div>
)
