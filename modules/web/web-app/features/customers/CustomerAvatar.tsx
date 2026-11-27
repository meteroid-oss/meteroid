import { cn } from '@md/ui'

/** The first letters of a customer's first two words: "Northwind Collective" → "NC". */
export const customerInitials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map(word => word[0]?.toUpperCase())
    .join('')

const SIZES = {
  // A list row (Home's top customers).
  sm: 'size-6 rounded-[5px] text-[10px]',
  // The customer page's title.
  lg: 'size-14 rounded-xl text-lg',
}

/** A customer's initials in a neutral tile, the same wherever a customer is shown. */
export const CustomerAvatar = ({
  name,
  size = 'sm',
  className,
}: {
  name: string
  size?: keyof typeof SIZES
  className?: string
}) => (
  <div
    aria-hidden
    className={cn(
      'flex shrink-0 items-center justify-center border border-border bg-foreground/[0.05] font-display font-medium leading-none text-foreground/80',
      SIZES[size],
      className
    )}
  >
    {customerInitials(name) || '?'}
  </div>
)
