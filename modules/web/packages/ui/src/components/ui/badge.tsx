import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const badgeVariants = cva(
  'inline-flex items-center justify-center w-fit whitespace-nowrap shrink-0 rounded-md border text-xs font-semibold gap-1 transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 [&>svg]:size-3 [&>svg]:pointer-events-none',
  {
    variants: {
      variant: {
        default: 'border-transparent bg-brand text-brand-foreground shadow hover:bg-brand/80',
        warning: 'border-transparent bg-warning text-warning-foreground shadow hover:bg-warning/80',
        success: 'border-transparent bg-success text-success-foreground shadow hover:bg-success/80',
        secondary:
          'border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80',
        destructive:
          'border-transparent bg-destructive text-destructive-foreground shadow hover:bg-destructive/80',
        outline: 'text-foreground',
        ghost: 'border-dashed border-muted-foreground/50 text-muted-foreground bg-transparent',
      },
      size: {
        sm: 'px-1.5 py-0 text-xs',
        md: 'px-2.5 py-1',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'md',
    },
  }
)

export interface BadgeProps
  extends React.HTMLAttributes<HTMLElement>,
    VariantProps<typeof badgeVariants> {
  asChild?: boolean
}

function Badge({ className, variant, size, asChild = false, ...props }: BadgeProps) {
  const Comp = asChild ? Slot : 'span'
  return (
    <Comp data-slot="badge" className={cn(badgeVariants({ variant, size }), className)} {...props} />
  )
}

export { Badge, badgeVariants }
