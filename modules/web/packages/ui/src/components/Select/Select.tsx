import { CheckIcon } from '@heroicons/react/24/solid'
import { colors } from '@md/foundation'
import { ChevronDownIcon } from '@md/icons'
import * as SelectPrimitive from '@radix-ui/react-select'
import * as React from 'react'

import { pipe } from '@ui/internal'
import { cn } from '@ui/lib/cn'

import {
  StyledContent,
  StyledItem,
  StyledItemIndicator,
  StyledTrigger,
  StyledViewport,
} from './Select.styled'

const SelectRoot = SelectPrimitive.Root

const SelectGroup = SelectPrimitive.Group

const SelectValue = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Value>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Value>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Value
    ref={ref}
    className={cn('placeholder:text-scale-800', className)}
    {...props}
  />
))
SelectValue.displayName = SelectPrimitive.Value.displayName

// TODO merge
// >(({ className, children, ...props }, ref) => (
//   <SelectPrimitive.Trigger
//     ref={ref}
//     className={cn(
//       'flex h-10 w-full items-center justify-between rounded-md border dark:border-scale-700  dark:bg-gray-300 px-3 py-2 text-sm  data-[placeholder]:text-scale-800 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50 focus:border-scale-900 focus:ring-scale-400',
//       className
//     )}
//     {...props}
//   ></SelectPrimitive.Trigger>
const SelectTrigger = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>
>(({ children, ...props }, ref) => (
  <StyledTrigger ref={ref} {...props}>
    {children}
    <SelectPrimitive.Icon asChild>
      <ChevronDownIcon size={14} fill={colors.neutral9} />
    </SelectPrimitive.Icon>
  </StyledTrigger>
))
SelectTrigger.displayName = SelectPrimitive.Trigger.displayName

const SelectContent = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content> & { viewportClassName?: string }
>(({ className, children, position = 'popper', viewportClassName, ...props }, ref) => (
  <SelectPrimitive.Portal>
    <StyledContent
      ref={ref}
      className={cn(
        'relative z-50 overflow-hidden ',
        position === 'popper' && 'translate-y-1',
        className
      )}
      position={position}
      {...props}
    >
      <StyledViewport
        className={cn(
          'p-1 ',
          position === 'popper' &&
            'h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)]',
          viewportClassName
        )}
      >
        {children}
      </StyledViewport>
    </StyledContent>
  </SelectPrimitive.Portal>
))
SelectContent.displayName = SelectPrimitive.Content.displayName

const SelectLabel = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Label
    ref={ref}
    className={cn('py-1.5 pl-8 pr-2 text-sm font-semibold', className)}
    {...props}
  />
))
SelectLabel.displayName = SelectPrimitive.Label.displayName

const SelectItem = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item> & { badge?: React.ReactNode }
>(({ className, badge, children, ...props }, ref) => (
  <StyledItem
    ref={ref}
    className={cn(
      'relative flex w-full select-none items-center outline-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
      className
    )}
    {...props}
  >
    <StyledItemIndicator>
      <CheckIcon className="h-4 w-4" fill={colors.neutral12} />
    </StyledItemIndicator>
    {pipe(<SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>, text =>
      badge ? (
        <div className="flex w-full justify-between">
          <div>{text}</div>
          {badge}
        </div>
      ) : (
        text
      )
    )}
  </StyledItem>
))
SelectItem.displayName = SelectPrimitive.Item.displayName

const SelectSeparator = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Separator>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Separator
    ref={ref}
    className={cn('-mx-1 my-1 h-px bg-muted', className)}
    {...props}
  />
))
SelectSeparator.displayName = SelectPrimitive.Separator.displayName

interface SelectProps {
  className?: string
  placeholder?: string
  size?: 'tiny' | 'small' | 'medium' | 'large'
  children: React.ReactNode
  onChange?: (value: { target: unknown; type?: unknown }) => void
}

const Select = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Root> & SelectProps
>(({ onChange, className, children, onValueChange, ...props }, forwardedRef) => {
  const mappedChange = (value: string) => {
    if (onValueChange) return onValueChange(value)
    if (onChange) return onChange({ target: { value, name: props.name } })
  }

  return (
    <SelectPrimitive.Root {...props} onValueChange={mappedChange}>
      <SelectTrigger {...props} ref={forwardedRef} className={className}>
        <SelectValue {...props} placeholder={props.placeholder} />
      </SelectTrigger>
      <SelectContent>{children}</SelectContent>
    </SelectPrimitive.Root>
  )
})

export default SelectItem

export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectRoot,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
}
