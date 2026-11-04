import { Collapsible, CollapsibleContent, CollapsibleTrigger, cn } from '@md/ui'
import { ChevronDown } from 'lucide-react'

import type { ReactNode } from 'react'

const cardClass = 'mb-6 rounded-lg border border-border bg-card shadow-sm'

const Heading = ({ title, subtitle }: { title: ReactNode; subtitle?: ReactNode }) => (
  <div className="min-w-0">
    <h3 className="text-md font-medium text-foreground">{title}</h3>
    {subtitle && <div className="mt-0.5 text-xs text-muted-foreground">{subtitle}</div>}
  </div>
)

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
    <Collapsible defaultOpen={defaultOpen} className={cn(cardClass, className)}>
      <CollapsibleTrigger className="group flex w-full items-start justify-between gap-4 p-4 text-left">
        <Heading title={title} subtitle={subtitle} />
        <div className="flex shrink-0 items-start gap-3">
          {aside}
          <ChevronDown className="mt-1 h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
        </div>
      </CollapsibleTrigger>
      <CollapsibleContent className="border-t border-border">{children}</CollapsibleContent>
    </Collapsible>
  ) : (
    <section className={cn(cardClass, className)}>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 border-b border-border p-4">
        <Heading title={title} subtitle={subtitle} />
        {aside}
      </div>
      {children}
    </section>
  )
