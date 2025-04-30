import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const buttonVariants = cva(
  'inline-flex items-center justify-center whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        brand: 'bg-brand text-brand-foreground shadow hover:bg-brand/90',
        primary: 'bg-primary text-primary-foreground shadow hover:bg-primary/80',
        default:
          'bg-brand dark:bg-primary text-brand-foreground dark:text-primary-foreground shadow hover:bg-brand/90',
        destructive: 'bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90',
        destructiveGhost:
          'text-destructive hover:text-destructive-foreground hover:bg-destructive/70',
        outline:
          'border border-border bg-background shadow-sm hover:bg-accent hover:text-accent-foreground',
        secondary: 'bg-secondary text-secondary-foreground shadow-sm hover:bg-accent',
        ghost: 'hover:bg-accent hover:text-accent-foreground',
        link: 'text-brand underline-offset-4 hover:underline',
        special:
          'border border-border bg-background dark:border-0 dark:bg-secondary hover:bg-accent dark:hover:bg-accent',
      },
      size: {
        md: 'h-9 px-4 py-2',
        content: '',
        full: 'h-full w-full',
        sm: 'h-8 rounded-md px-3 text-xs mt-0.5',
        lg: 'h-10 rounded-md px-8',
        icon: 'h-9 w-9',
      },
      hasIcon: {
        true: 'gap-2',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'md',
    },
  }
)

type ButtonVariants = VariantProps<typeof buttonVariants>

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, hasIcon, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : 'button'
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, hasIcon, className }))}
        ref={ref}
        {...props}
      />
    )
  }
)
Button.displayName = 'Button'

export { Button, buttonVariants, type ButtonVariants }
