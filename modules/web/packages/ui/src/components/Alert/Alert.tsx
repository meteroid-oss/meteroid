import { spaces } from '@md/foundation'
import * as React from 'react'

import { Flex } from '@ui/components/Flex'

import { StyledAlert, Title } from './Alert.styled'

import type * as Stitches from '@stitches/react'

const Alert = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement> &
    Stitches.VariantProps<typeof StyledAlert> & {
      title?: string
    }
>(({ title, children, ...props }, ref) => (
  <StyledAlert ref={ref} role="alert" {...props}>
    <Flex direction="column" gap={spaces.space2}>
      {title && <Title>{title}</Title>}
      {children}
    </Flex>
  </StyledAlert>
))
Alert.displayName = 'Alert'

export { Alert }
