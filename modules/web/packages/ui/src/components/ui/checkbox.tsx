import * as CheckboxPrimitive from '@radix-ui/react-checkbox'
import * as React from 'react'

import { cn } from '@ui/lib'

/**
 * Both marks stay in the DOM (rather than in a Checkbox.Indicator, which Radix unmounts on
 * uncheck) so they can draw in and erase out; `mt-check` in main.css shows the one that matches
 * `aria-checked`: the tick for true, the dash for "mixed" (indeterminate).
 */
const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => (
  <CheckboxPrimitive.Root
    ref={ref}
    className={cn('mt-check peer disabled:cursor-not-allowed disabled:opacity-50', className)}
    {...props}
  >
    <svg viewBox="0 0 12 12" aria-hidden="true" className="mt-check-mark">
      <path pathLength={1} d="M2.5 6.5 5 9l4.5-6" />
    </svg>
    <svg viewBox="0 0 12 12" aria-hidden="true" className="mt-check-dash">
      <path pathLength={1} d="M3 6h6" />
    </svg>
  </CheckboxPrimitive.Root>
))
Checkbox.displayName = CheckboxPrimitive.Root.displayName

export { Checkbox }
