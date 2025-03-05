import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const badgeVariants = cva(
  'inline-flex items-center rounded-md border text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2',
  {
    variants: {
      variant: {
        default: 'border-transparent bg-brand text-brand-foreground shadow hover:bg-brand/80',
        brand: 'border-transparent bg-brand text-brand-foreground shadow hover:bg-brand/80',
        warning: 'border-transparent bg-warning text-warning-foreground shadow hover:bg-warning/80',
        success: 'border-transparent bg-success text-success-foreground shadow hover:bg-success/80',
        primary: 'border-transparent bg-primary text-primary-foreground shadow hover:bg-primary/80',
        secondary:
          'border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80',
        destructive:
          'border-transparent bg-destructive text-destructive-foreground shadow hover:bg-destructive/80',
        outline: 'text-foreground',
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
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, size, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant, size }), className)} {...props} />
}

export { Badge, badgeVariants }
