import * as LabelPrimitive from '@radix-ui/react-label'
import * as React from 'react'

import { StyledLabel } from '@ui/components/Label/Label.styled'

const Label = React.forwardRef<
  React.ElementRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => <StyledLabel ref={ref} className={className} {...props} />)
Label.displayName = LabelPrimitive.Root.displayName

export { Label }
