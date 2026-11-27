import * as React from 'react'

import { cn } from '@ui/lib'

export interface TextareaProps extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          'flex min-h-[60px] w-full rounded-md border border-field-border bg-field px-3 py-2 text-[13px] text-field-foreground placeholder:text-field-placeholder focus-visible:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8 transition-[border-color,box-shadow] disabled:cursor-not-allowed disabled:opacity-50',
          className
        )}
        ref={ref}
        {...props}
      />
    )
  }
)
Textarea.displayName = 'Textarea'

export { Textarea }
