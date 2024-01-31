import * as SwitchPrimitives from '@radix-ui/react-switch'
import * as React from 'react'

import { Flex } from '@ui/components/Flex'

import { Label, SwitchRoot, SwitchThumb } from './Switch.styled'

const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root> & {
    id: string
  }
>(({ id, ...props }, ref) => (
  <Flex align="center">
    <Label htmlFor={id} css={{ paddingRight: 15 }}>
      Airplane mode
    </Label>
    <SwitchRoot ref={ref} id={id} {...props}>
      <SwitchThumb />
    </SwitchRoot>
  </Flex>
))
Switch.displayName = SwitchPrimitives.Root.displayName

export { Switch }
