import { Collapsible, CollapsibleContent, CollapsibleTrigger, cn } from '@md/ui'
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { get, useFormState } from 'react-hook-form'

import type { ReactNode } from 'react'
import type { Control, FieldPath, FieldValues } from 'react-hook-form'

type Props = {
  /** Open on first render, e.g. when a field inside holds a non-default value. */
  defaultOpen?: boolean
  /** Opens the section and keeps it open while true, e.g. on a validation error inside. */
  forceOpen?: boolean
  label?: string
  className?: string
  children: ReactNode
}

/** Optional fields folded under a collapsible "Advanced settings" toggle. */
export const AdvancedSection = ({
  defaultOpen = false,
  forceOpen = false,
  label = 'Advanced settings',
  className,
  children,
}: Props) => {
  const [open, setOpen] = useState(defaultOpen || forceOpen)
  if (forceOpen && !open) setOpen(true)

  return (
    <Collapsible open={open} onOpenChange={setOpen} className={className}>
      <CollapsibleTrigger
        type="button"
        className="flex items-center gap-1 text-[13px] font-medium text-muted-foreground hover:text-foreground"
      >
        <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} />
        {label}
      </CollapsibleTrigger>
      <CollapsibleContent className="space-y-4 pt-3">{children}</CollapsibleContent>
    </Collapsible>
  )
}

/** Whether any of these fields (or a field nested under them) has a validation error. */
export const useHasFieldErrors = <T extends FieldValues>(
  control: Control<T>,
  names: readonly FieldPath<T>[]
) => {
  const { errors } = useFormState({ control, name: [...names] })
  return names.some(name => get(errors, name) !== undefined)
}
