import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const badgeVariants = cva(
  // Tags: a solid fill with text in the same tone (the `--tag-*` tokens, from the Figma tags).
  'inline-flex items-center justify-center w-fit whitespace-nowrap shrink-0 rounded-[4px] border border-transparent text-xs font-medium leading-none gap-1 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring [&>svg]:size-3 [&>svg]:pointer-events-none',
  {
    variants: {
      variant: {
        // The design's blue tag ("Active", "Upcoming"): #3dd7d7 on #153e4c at 70%.
        default: 'bg-tag-info-bg/70 text-tag-info',
        info: 'bg-tag-info-bg/70 text-tag-info',
        success: 'bg-tag-success-bg text-tag-success',
        warning: 'bg-tag-warning-bg text-tag-warning',
        destructive: 'bg-tag-destructive-bg text-tag-destructive',
        secondary: 'bg-tag-neutral-bg text-tag-neutral',
        outline: 'border-border text-foreground',
        ghost: 'border-dashed border-muted-foreground/50 text-muted-foreground bg-transparent',
      },
      size: {
        sm: 'h-[18px] px-1 text-[11px]',
        md: 'h-6 px-1.5',
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
