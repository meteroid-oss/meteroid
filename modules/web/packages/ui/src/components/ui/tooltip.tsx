import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'

import { cn } from '@ui/lib'

/** Tooltips open 80ms after hover or focus; moving on to the next trigger skips the delay. */
const TooltipProvider = ({
  delayDuration = 80,
  ...props
}: React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Provider>) => (
  <TooltipPrimitive.Provider delayDuration={delayDuration} {...props} />
)
TooltipProvider.displayName = TooltipPrimitive.Provider.displayName

const Tooltip = TooltipPrimitive.Root

const TooltipTrigger = TooltipPrimitive.Trigger

const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content> & {
    /** Let longer text wrap (up to `max-w-xs`, or a `max-w-*` in `className`). By default a
     *  tooltip is a single line. */
    multiline?: boolean
  }
>(({ className, sideOffset = 4, multiline, ...props }, ref) => (
  <TooltipPrimitive.Content
    ref={ref}
    sideOffset={sideOffset}
    className={cn(
      'mt-tooltip z-50 text-xs',
      multiline && 'max-w-xs whitespace-normal break-words',
      className
    )}
    {...props}
  />
))
TooltipContent.displayName = TooltipPrimitive.Content.displayName

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider }
