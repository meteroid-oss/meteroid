import * as PopoverPrimitive from '@radix-ui/react-popover'
import * as React from 'react'

import { cn } from '@ui/lib'

import { menuSurface } from './menu-styles'

const Popover = PopoverPrimitive.Root

const PopoverTrigger = PopoverPrimitive.Trigger

const PopoverAnchor = PopoverPrimitive.Anchor

const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content> & {
    useTriggerWidth?: boolean
  }
>(
  (
    // Kept 12px off the window's edges when it has to shift to fit, rather than flush against them.
    {
      className,
      useTriggerWidth,
      align = 'center',
      sideOffset = 4,
      collisionPadding = 12,
      ...props
    },
    ref
  ) => (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        ref={ref}
        collisionPadding={collisionPadding}
        align={align}
        sideOffset={sideOffset}
        className={cn(
          'mt-dropdown z-50 w-72 p-4 outline-none',
          menuSurface,
          useTriggerWidth && 'w-[var(--radix-popover-trigger-width)]',
          className
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  )
)
PopoverContent.displayName = PopoverPrimitive.Content.displayName

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor }
