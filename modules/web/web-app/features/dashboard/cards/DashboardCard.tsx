import { cn } from '@md/ui'

import type { ReactNode } from 'react'

/**
 * The Home page's card surface: 6px corners, 20px padding. `raised` adds the stat cards' 1px
 * drop shadow. A 0.5px border at 12%, like the list pages' table cards.
 */
export const DashboardCard = ({
  raised,
  className,
  children,
}: {
  raised?: boolean
  className?: string
  children: ReactNode
}) => (
  <div
    className={cn(
      // Reference surfaces: #1e1e1e for content cards, #1a1a1a for stat cards.
      'mt-dashboard-card relative rounded-[6px] border-[0.5px] border-foreground/12 bg-card p-5 dark:bg-[#1e1e1e]',
      raised && 'drop-shadow-[0_1px_0_rgba(0,0,0,0.25)] dark:bg-[#1a1a1a]',
      className
    )}
  >
    {children}
  </div>
)

/**
 * A link in a card (a customer's name, a tile's title): a fine dashed underline at 20%,
 * firmer on hover. With `truncate`, give the line some height (e.g. `leading-5`): the underline
 * sits below the text and is clipped with it.
 */
export const dashedLink =
  'underline decoration-foreground/20 decoration-dashed decoration-1 underline-offset-[3px] transition-colors hover:decoration-foreground/45'

/** A card's small title: "Revenue", "MRR Breakdown", "Active subscriptions"… */
export const DashboardCardTitle = ({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) => (
  <div className={cn('text-[13px] leading-none tracking-[-0.13px] text-foreground/60', className)}>
    {children}
  </div>
)
