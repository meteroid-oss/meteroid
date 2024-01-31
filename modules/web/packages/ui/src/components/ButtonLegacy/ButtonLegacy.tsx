import { VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib/cn'

import { buttonVariants } from './ButtonLegacy.styled'

export interface ButtonLegacyProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

const ButtonLegacy = React.forwardRef<HTMLButtonElement, ButtonLegacyProps>(
  ({ className, variant, size, ...props }, ref) => {
    return (
      <button className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
    )
  }
)
ButtonLegacy.displayName = 'ButtonLegacy'

export { ButtonLegacy, buttonVariants }
