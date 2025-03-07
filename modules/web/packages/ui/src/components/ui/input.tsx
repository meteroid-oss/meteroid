import { Eye, EyeOff } from 'lucide-react'
import * as React from 'react'

import { cn } from '@ui/lib'

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  rightText?: string
  wrapperClassName?: string
  showPasswordToggle?: boolean
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, wrapperClassName, type, rightText, showPasswordToggle, ...props }, ref) => {
    const [showPassword, setShowPassword] = React.useState(false)

    const inputElement = (
      <input
        type={showPassword ? 'text' : type}
        className={cn(
          'flex h-9 w-full rounded-md border border-border bg-input px-3 py-1 text-sm shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:bg-muted',
          rightText && 'rounded-r-none',
          showPasswordToggle && 'pr-10',
          showPasswordToggle && !showPassword && 'password', // chrome has a weird behavior with type=password and will ignore autocomplete=off. This is a workaround
          className
        )}
        ref={ref}
        {...props}
      />
    )

    const renderContent = () => {
      if (showPasswordToggle) {
        return (
          <div className="relative w-full">
            {inputElement}
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-accent-foreground"
            >
              {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </button>
          </div>
        )
      }

      if (rightText) {
        return (
          <div className={cn('flex', wrapperClassName)}>
            {inputElement}
            <div className="border border-border border-l-0 rounded-md rounded-l-none self-end h-9 text-sm px-2 content-center bg-muted text-muted-foreground">
              {rightText}
            </div>
          </div>
        )
      }

      return inputElement
    }

    return renderContent()
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
