import { Collapsible, CollapsibleContent, CollapsibleTrigger, cn } from '@md/ui'
import { ChevronDown } from 'lucide-react'

import { containedTable } from '@/features/customers/cards/ContainedTableSection'

import type { ReactNode } from 'react'

// The content's frame: as the customer page's tables, its fill a step below the surface, and a
// table in it drawn as theirs.
const boxClass = cn('overflow-hidden rounded-lg border border-border bg-card', containedTable)

const Heading = ({ title, subtitle }: { title: ReactNode; subtitle?: ReactNode }) => (
  <div className="flex min-w-0 flex-col gap-0.5">
    <h3 className="text-base font-medium leading-7 text-foreground">{title}</h3>
    {subtitle && <div className="text-[13px] leading-snug text-muted-foreground">{subtitle}</div>}
  </div>
)

/**
 * A section of the subscription page, as the sections of the other detail pages: a 16px title
 * (with a line below it and something on the right), then the content in a bordered box 16px
 * below. Spacing between sections is the page's (see `DetailSections`). Collapsible sections
 * fold the box away from their heading.
 */
export const SubscriptionSectionCard = ({
  title,
  subtitle,
  aside,
  children,
  className,
  collapsible = false,
  defaultOpen = true,
}: {
  title: ReactNode
  subtitle?: ReactNode
  aside?: ReactNode
  children: ReactNode
  className?: string
  collapsible?: boolean
  defaultOpen?: boolean
}) =>
  collapsible ? (
    <Collapsible defaultOpen={defaultOpen} className={cn('flex flex-col', className)}>
      <CollapsibleTrigger className="group flex w-full items-start justify-between gap-4 text-left">
        <Heading title={title} subtitle={subtitle} />
        <div className="flex shrink-0 items-center gap-3 pt-1">
          {aside}
          <ChevronDown className="h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
        </div>
      </CollapsibleTrigger>
      <CollapsibleContent className={cn('mt-4', boxClass)}>{children}</CollapsibleContent>
    </Collapsible>
  ) : (
    <section className={cn('flex flex-col gap-4', className)}>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1">
        <Heading title={title} subtitle={subtitle} />
        {aside}
      </div>
      <div className={boxClass}>{children}</div>
    </section>
  )
