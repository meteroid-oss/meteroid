import * as TogglePrimitive from '@radix-ui/react-toggle'
import * as React from 'react'

import { StyledToggle } from './Toggle.styled'

import type * as Stitches from '@stitches/react'

const Toggle = React.forwardRef<
  React.ElementRef<typeof TogglePrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof TogglePrimitive.Root> &
    Stitches.VariantProps<typeof StyledToggle>
>(({ ...props }, ref) => <StyledToggle ref={ref} {...props} />)

Toggle.displayName = TogglePrimitive.Root.displayName

export { Toggle }
