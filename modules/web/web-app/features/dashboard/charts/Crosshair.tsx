import { cn } from '@ui/lib'
import { forwardRef } from 'react'

const CrosshairLine = forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn('z-10 absolute h-full w-px bg-muted-foreground pointer-events-none', className)}
      {...props}
    />
  )
)
CrosshairLine.displayName = 'CrosshairLine'

const CrosshairTooltip = forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        'z-10 absolute top-0 left-0 text-sm bg-opacity-90 bg-popover text-popover-foreground shadow-lg rounded-md py-2 px-4 pointer-events-none',
        className
      )}
      {...props}
    />
  )
)
CrosshairTooltip.displayName = 'CrosshairTooltip'

const CrosshairPoint = forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        'z-10 absolute h-3 w-3 bg-primary rounded-full transform -translate-x-1/2 -translate-y-1/2 pointer-events-none',
        className
      )}
      {...props}
    />
  )
)
CrosshairPoint.displayName = 'CrosshairPoint'

export const Crosshair = {
  Line: CrosshairLine,
  Tooltip: CrosshairTooltip,
  Point: CrosshairPoint,
}
