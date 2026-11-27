import * as HoverCardPrimitive from '@radix-ui/react-hover-card'
import * as React from 'react'

import { cn } from '@ui/lib'

const HoverCard = HoverCardPrimitive.Root

const HoverCardTrigger = HoverCardPrimitive.Trigger

const HoverCardContent = React.forwardRef<
  React.ElementRef<typeof HoverCardPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof HoverCardPrimitive.Content>
>(({ className, align = 'center', sideOffset = 4, ...props }, ref) => (
  // In a portal, so a scrolling or clipped parent (the sidebar) can't cut it off.
  <HoverCardPrimitive.Portal>
    <HoverCardPrimitive.Content
      ref={ref}
      align={align}
      sideOffset={sideOffset}
      className={cn(
        'mt-dropdown z-50 w-64 rounded-md border bg-popover p-4 text-popover-foreground shadow-md outline-none',
        className
      )}
      {...props}
    />
  </HoverCardPrimitive.Portal>
))
HoverCardContent.displayName = HoverCardPrimitive.Content.displayName

export { HoverCard, HoverCardTrigger, HoverCardContent }
