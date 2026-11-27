import { cn } from '@md/ui'
import { ReactNode, useId } from 'react'

/** A card of the quote page's side rail. */
export const RailCard = ({
  title,
  aside,
  children,
  className,
}: {
  title: string
  aside?: ReactNode
  children: ReactNode
  className?: string
}) => {
  const headingId = useId()
  return (
    <section
      aria-labelledby={headingId}
      className={cn('flex flex-col gap-3 rounded-lg border border-border bg-card p-4', className)}
    >
      <div className="flex items-center justify-between gap-2">
        <h2 id={headingId} className="text-[15px] font-medium text-foreground">
          {title}
        </h2>
        {aside}
      </div>
      {children}
    </section>
  )
}
