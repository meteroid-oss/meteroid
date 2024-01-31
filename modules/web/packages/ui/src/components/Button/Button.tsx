import * as React from 'react'

import { Dots } from '@ui/components/Dots'

import { StyledButton } from './Button.styled'

export type ButtonProps = React.ComponentProps<typeof StyledButton> & {
  loading?: boolean
  fullWidth?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ loading, fullWidth, ...props }, ref) => {
    const { children, variant } = props
    const dotsVariant =
      variant === 'primary' ||
      variant === 'secondary' ||
      variant === 'success' ||
      variant === 'danger'
        ? 'light'
        : 'dark'

    const isIcon =
      React.Children.count(children) === 1 &&
      typeof children === 'object' &&
      children !== null &&
      Object.prototype.hasOwnProperty.call(children, '$$typeof')

    return (
      <StyledButton
        ref={ref}
        icon={isIcon}
        style={{
          width: fullWidth ? '100%' : 'fit-content',
          ...props.style,
        }}
        {...props}
      >
        {loading ? <Dots variant={dotsVariant} size="small" /> : children}
      </StyledButton>
    )
  }
)

Button.displayName = 'Button'

export { Button }
