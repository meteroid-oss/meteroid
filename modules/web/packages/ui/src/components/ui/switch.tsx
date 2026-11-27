import * as SwitchPrimitives from '@radix-ui/react-switch'
import * as React from 'react'

import { cn } from '@ui/lib'

/** Look and motion come from `mt-switch` / `mt-switch-thumb` in main.css. */
const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root> & {
    thumbClassName?: string
  }
>(({ className, thumbClassName, ...props }, ref) => (
  <SwitchPrimitives.Root
    className={cn('mt-switch peer disabled:cursor-not-allowed disabled:opacity-50', className)}
    {...props}
    ref={ref}
  >
    <SwitchPrimitives.Thumb
      className={cn('mt-switch-thumb pointer-events-none block', thumbClassName)}
    />
  </SwitchPrimitives.Root>
))
Switch.displayName = SwitchPrimitives.Root.displayName

export { Switch }
