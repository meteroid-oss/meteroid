import { CheckIcon } from '@heroicons/react/24/solid'
import { colors } from '@md/foundation'
import * as CheckboxPrimitive from '@radix-ui/react-checkbox'
import * as React from 'react'

import { StyledRoot } from '@ui/components/Checkbox/Checkbox.styled'
import { cn } from '@ui/lib/cn'

const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => (
  <StyledRoot
    ref={ref}
    className={cn(
      'peer h-4 w-4 shrink-0 disabled:cursor-not-allowed disabled:opacity-50 ',
      className
    )}
    {...props}
  >
    <CheckboxPrimitive.Indicator className={cn('flex items-center justify-center ')}>
      <CheckIcon className="h-3 w-3 " fill={colors.white1} />
    </CheckboxPrimitive.Indicator>
  </StyledRoot>
))
Checkbox.displayName = CheckboxPrimitive.Root.displayName

export { Checkbox }
