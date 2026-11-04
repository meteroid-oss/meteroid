import { cn } from '@md/ui'
import { ReactNode } from 'react'

import type { LucideIcon } from 'lucide-react'

/**
 * An empty, not-run or error state: a muted icon, a 14px medium title, a 13px muted line, an
 * optional action. Centered in its container. Use for lists, panes and tables alike.
 * `quiet` is for a placeholder that only says where to look (an empty detail pane): the title
 * drops to a 13px muted line so it doesn't compete with the content beside it.
 */
export const PanelEmptyState = ({
  icon: Icon,
  title,
  description,
  action,
  tone = 'muted',
  quiet = false,
  className,
}: {
  icon?: LucideIcon
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
  /** `destructive` only for a load error. */
  tone?: 'muted' | 'destructive'
  /** A hint rather than a state: muted 13px title, no weight. */
  quiet?: boolean
  className?: string
}) => (
  <div className={cn('flex flex-col items-center gap-2 px-4 py-12 text-center', className)}>
    {Icon && (
      <Icon
        className={cn(
          'size-5',
          tone === 'destructive' ? 'text-destructive' : 'text-muted-foreground'
        )}
        aria-hidden
      />
    )}
    <p
      className={cn(
        quiet ? 'text-[13px] text-muted-foreground' : 'text-sm font-medium text-foreground'
      )}
    >
      {title}
    </p>
    {description && <p className="max-w-md text-[13px] text-muted-foreground">{description}</p>}
    {action && <div className="pt-1">{action}</div>}
  </div>
)
