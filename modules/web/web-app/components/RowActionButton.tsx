import { Button, cn } from '@md/ui'
import { ComponentProps, forwardRef } from 'react'

/**
 * An action on a list row (edit, pin, toggle, remove): a small secondary icon button. Its icon is
 * quiet, brightening on hover; a destructive action keeps its icon red. Name it with
 * `aria-label`.
 */
export const RowActionButton = forwardRef<
  HTMLButtonElement,
  Omit<ComponentProps<typeof Button>, 'variant' | 'size'> & { destructive?: boolean }
>(({ destructive, className, ...props }, ref) => (
  <Button
    ref={ref}
    type="button"
    variant="secondary"
    size="sm"
    className={cn(
      'size-7 px-0 [&_svg]:size-3.5',
      destructive
        ? 'text-tag-destructive hover:text-tag-destructive'
        : 'text-muted-foreground hover:text-foreground',
      className
    )}
    {...props}
  />
))
RowActionButton.displayName = 'RowActionButton'
