import { forwardRef } from 'react'

import {
  Input as InputComponent,
  InputContainer,
  InputIcon,
} from '@ui/components/Input/Input.styled'

import type { InputHTMLAttributes } from 'react'

export type InputProps = InputHTMLAttributes<HTMLInputElement> & {
  width?: string | number
  icon?: JSX.Element
  iconPosition?: 'left' | 'right'
}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ width = '100%', icon, iconPosition = 'right', className, style, ...props }, ref) => {
    return (
      <InputContainer
        style={{
          width,
        }}
      >
        <InputComponent
          className={className}
          ref={ref}
          icon={iconPosition}
          style={{
            ...style,
            width,
          }}
          {...props}
        />
        {icon && <InputIcon position={iconPosition}>{icon}</InputIcon>}
      </InputContainer>
    )
  }
)
Input.displayName = 'Input'
