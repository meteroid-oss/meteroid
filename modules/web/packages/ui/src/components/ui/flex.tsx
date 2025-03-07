import { cn } from '@ui/lib'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

const flexVariants = cva('flex transition-all', {
  variants: {
    display: {
      flex: 'flex',
      inline: 'inline-flex',
    },
    direction: {
      row: 'flex-row',
      column: 'flex-col',
      rowReverse: 'flex-row-reverse',
      columnReverse: 'flex-col-reverse',
    },
    align: {
      start: 'items-start',
      center: 'items-center',
      end: 'items-end',
      stretch: 'items-stretch',
    },
    justify: {
      start: 'justify-start',
      end: 'justify-end',
      center: 'justify-center',
      between: 'justify-between',
      around: 'justify-around',
      evenly: 'justify-evenly',
    },
  },
  defaultVariants: {
    display: 'flex',
    direction: 'row',
    align: 'stretch',
    justify: 'start',
  },
})

export interface FlexProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof flexVariants> {}

const Flex = React.forwardRef<HTMLDivElement, FlexProps>(
  ({ className, direction, align, justify, ...props }, ref) => {
    return (
      <div
        className={cn(flexVariants({ direction, align, justify, className }))}
        ref={ref}
        {...props}
      />
    )
  }
)

Flex.displayName = 'Flex'

export { Flex, flexVariants }
