import * as React from 'react'

import { cn } from '@ui/lib'

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  rightText?: string
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    const inputElement = (
      <input
        type={type}
        className={cn(
          'flex h-9 w-full rounded-md border border-border bg-input px-3 py-1 text-sm shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:bg-muted',
          props.rightText && 'rounded-r-none',
          className
        )}
        ref={ref}
        {...props}
      />
    )

    return props.rightText ? (
      <div className="flex">
        {inputElement}
        <div className="border border-border border-l-0 rounded-md rounded-l-none self-end h-9 text-sm px-2 content-center bg-muted text-muted-foreground">
          {props.rightText}
        </div>
      </div>
    ) : (
      inputElement
    )
  }
)
Input.displayName = 'Input'

export interface InputWithIconProps extends React.InputHTMLAttributes<HTMLInputElement> {
  icon: React.ReactNode
  containerClassName?: string
}

const InputWithIcon = React.forwardRef<HTMLInputElement, InputWithIconProps>(
  ({ containerClassName, className, icon, ...props }, ref) => {
    return (
      <div
        className={cn('w-full relative', containerClassName)}
        style={{
          width: props.width,
        }}
      >
        <Input ref={ref} {...props} className={cn('pr-14', className)} />
        {icon && (
          <span className="absolute top-1/2 transform -translate-y-1/2 right-4 pointer-events-none">
            {icon}
          </span>
        )}
      </div>
    )
  }
)
InputWithIcon.displayName = 'InputWithIcon'

export { Input, InputWithIcon }
