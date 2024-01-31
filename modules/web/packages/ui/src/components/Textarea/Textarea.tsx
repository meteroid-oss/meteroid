import * as React from 'react'

import { StyledTextarea } from './Textarea.styled'
import { cn } from '@ui/lib'

export type TextareaProps = React.TextareaHTMLAttributes<HTMLTextAreaElement>

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => {
    // <<<<<<< Updated upstream TODO merge
    // return <StyledTextarea className={className} ref={ref} {...props} />
    // =======
    return (
      <textarea
        className={cn(
          'flex h-20 w-full rounded-md border border-slate-600  bg-scaleA-200 py-2 px-3 text-sm placeholder:text-slate-800 focus:outline-none focus:ring-2 focus:ring-slate-400 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 dark:border-slate-700 dark:text-slate-50 dark:focus:ring-slate-400 dark:focus:ring-offset-slate-900',
          className
        )}
        ref={ref}
        {...props}
      />
    )
    // >>>>>>> Stashed changes
  }
)
Textarea.displayName = 'Textarea'

export { Textarea }
