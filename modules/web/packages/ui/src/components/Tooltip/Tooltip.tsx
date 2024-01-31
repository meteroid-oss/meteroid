import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import * as React from 'react'

import { StyledTooltipArrow, StyledTooltipContent } from './Tooltip.styled'

const TooltipProvider = TooltipPrimitive.Provider

const Tooltip: React.FC<TooltipPrimitive.TooltipProps> = ({ ...props }) => (
  <TooltipPrimitive.Root {...props} />
)
Tooltip.displayName = TooltipPrimitive.Tooltip.displayName

const TooltipTrigger = TooltipPrimitive.Trigger
const TooltipPortal = TooltipPrimitive.Portal

const TooltipArrow = StyledTooltipArrow

const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 4, ...props }, ref) => (
  <StyledTooltipContent ref={ref} sideOffset={sideOffset} className={className} {...props} />
))
TooltipContent.displayName = TooltipPrimitive.Content.displayName

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider, TooltipArrow, TooltipPortal }
