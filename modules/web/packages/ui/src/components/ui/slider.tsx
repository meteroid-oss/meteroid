import * as SliderPrimitive from '@radix-ui/react-slider'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const sliderTrackVariants = cva('relative w-full grow overflow-hidden rounded-full', {
  variants: {
    variant: {
      default: 'h-1.5 bg-brand/20',
      neutral: 'h-1 bg-muted',
    },
  },
  defaultVariants: { variant: 'default' },
})

const sliderRangeVariants = cva('absolute h-full', {
  variants: {
    variant: {
      default: 'bg-brand',
      neutral: 'bg-muted-foreground/40',
    },
  },
  defaultVariants: { variant: 'default' },
})

const sliderThumbVariants = cva(
  'cursor-pointer block rounded-full transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default: 'h-4 w-4 border border-brand/50 bg-background shadow',
        neutral: 'h-4 w-4 bg-white shadow-md ring-0',
      },
    },
    defaultVariants: { variant: 'default' },
  }
)

const sliderTickVariants = cva('absolute block w-px -translate-x-1/2', {
  variants: {
    variant: {
      default: 'h-1.5 bg-border',
      neutral: 'h-2 bg-muted-foreground/40',
    },
  },
  defaultVariants: { variant: 'default' },
})

type SliderVariant = VariantProps<typeof sliderTrackVariants>

// Radix clamps the thumb to the track via a linear in-bounds offset.
// Match it so ticks/labels align with thumb centers at the extremes.
// thumbWidth=16 matches sliderThumbVariants (h-4/w-4).
const THUMB_WIDTH_PX = 16
export const getSliderTickPosition = (index: number, total: number): string => {
  if (total <= 1) return '0%'
  const pct = (index / (total - 1)) * 100
  const offsetPx = ((50 - pct) / 100) * THUMB_WIDTH_PX
  return `calc(${pct}% + ${offsetPx}px)`
}

const Slider = React.forwardRef<
  React.ElementRef<typeof SliderPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SliderPrimitive.Root> &
    SliderVariant & {
      thumbClassName?: string
      trackClassName?: string
      rangeClassName?: string
      ticks?: number
      tickClassName?: string
    }
>(
  (
    {
      className,
      thumbClassName,
      trackClassName,
      rangeClassName,
      ticks,
      tickClassName,
      variant,
      ...props
    },
    ref
  ) => (
    <SliderPrimitive.Root
      ref={ref}
      className={cn('relative flex w-full touch-none select-none items-center', className)}
      {...props}
    >
      <SliderPrimitive.Track className={cn(sliderTrackVariants({ variant }), trackClassName)}>
        <SliderPrimitive.Range className={cn(sliderRangeVariants({ variant }), rangeClassName)} />
      </SliderPrimitive.Track>
      {ticks && ticks > 1 ? (
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-full mt-2 h-2">
          {Array.from({ length: ticks }).map((_, i) => (
            <span
              key={i}
              className={cn(sliderTickVariants({ variant }), tickClassName)}
              style={{ left: getSliderTickPosition(i, ticks) }}
            />
          ))}
        </div>
      ) : null}
      <SliderPrimitive.Thumb
        className={cn(sliderThumbVariants({ variant }), thumbClassName)}
        tabIndex={props.tabIndex}
      />
    </SliderPrimitive.Root>
  )
)
Slider.displayName = SliderPrimitive.Root.displayName

export { Slider }
