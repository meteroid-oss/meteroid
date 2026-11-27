import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const buttonVariants = cva(
  'inline-flex cursor-pointer items-center justify-center whitespace-nowrap rounded-[4px] text-[13px] font-medium transition-colors [&_svg]:shrink-0 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        brand: 'bg-brand text-brand-foreground shadow hover:bg-brand/90',
        primary: 'bg-primary text-primary-foreground hover:bg-primary/90',
        default:
          'bg-brand text-brand-foreground hover:bg-brand/90 dark:bg-primary dark:text-primary-foreground dark:hover:bg-primary/90',
        destructive: 'bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90',
        destructiveGhost:
          'text-destructive hover:text-destructive-foreground hover:bg-destructive/70',
        outline: 'border border-border bg-transparent hover:bg-accent hover:text-accent-foreground',
        secondary:
          'bg-secondary text-secondary-foreground shadow-sm hover:bg-accent dark:bg-foreground/[0.08] dark:shadow-none dark:hover:bg-foreground/[0.12]',
        ghost: 'hover:bg-accent hover:text-accent-foreground',
        link: 'text-brand underline-offset-4 hover:underline',
        flat: 'text-primary-foreground underline-offset-4 hover:underline',
        special:
          'border border-border bg-background dark:border-0 dark:bg-secondary hover:bg-accent dark:hover:bg-accent',
      },
      size: {
        md: 'h-9 px-4 py-2',
        content: '',
        full: 'h-full w-full',
        // The design's compact button: 28px, 13px text; with a leading icon, 8px left / 10px right.
        sm: 'h-7 px-2.5',
        lg: 'h-10 px-8',
        icon: 'h-9 w-9',
      },
      hasIcon: {
        true: 'gap-2',
      },
    },
    compoundVariants: [{ size: 'sm', hasIcon: true, class: 'gap-1.5 pl-2' }],
    defaultVariants: {
      variant: 'default',
      size: 'md',
    },
  }
)

type ButtonVariants = VariantProps<typeof buttonVariants>

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

/**
 * Whether the content starts with an icon followed by a label: an element that is not an HTML
 * tag (a lucide or radix icon, a spinner) or an svg, then anything more. Icon-only content is not.
 */
const startsWithIcon = (children: React.ReactNode): boolean => {
  const items = React.Children.toArray(children)
  const first = items[0]
  return (
    items.length > 1 &&
    React.isValidElement(first) &&
    (typeof first.type !== 'string' || first.type === 'svg')
  )
}

// A leading icon on a neutral button is a step quieter than its label, in a solid colour (a
// translucent one shows where the strokes cross). An icon given its own colour keeps it; filled
// buttons keep the icon in the label's colour.
const quietLeadingIcon = "[&>svg:first-child:not([class*='text-'])]:text-muted-foreground"
const NEUTRAL_VARIANTS = new Set(['secondary', 'outline', 'ghost', 'special'])

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, hasIcon, asChild = false, children, ...props }, ref) => {
    const Comp = asChild ? Slot : 'button'
    // With asChild the icon sits in the child (a Link), whose children are the content.
    const content =
      asChild && React.isValidElement<{ children?: React.ReactNode }>(children)
        ? children.props.children
        : children
    const quietIcon = !!variant && NEUTRAL_VARIANTS.has(variant) && startsWithIcon(content)
    return (
      <Comp
        className={cn(
          buttonVariants({ variant, size, hasIcon }),
          quietIcon && quietLeadingIcon,
          className
        )}
        ref={ref}
        {...props}
      >
        {children}
      </Comp>
    )
  }
)
Button.displayName = 'Button'

export { Button, buttonVariants, type ButtonVariants }
