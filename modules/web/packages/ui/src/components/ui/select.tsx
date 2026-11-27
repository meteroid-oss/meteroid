import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from '@radix-ui/react-icons'
import * as SelectPrimitive from '@radix-ui/react-select'
import * as React from 'react'

import SelectChevrons from '@ui/assets/select-chevrons.svg?react'
import { cn } from '@ui/lib'

import { menuItem, menuLabel, menuSeparator, menuSurface } from './menu-styles'

const Select = SelectPrimitive.Root

const SelectGroup = SelectPrimitive.Group

const SelectValue = SelectPrimitive.Value

const SelectTrigger = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>
>(({ className, children, ...props }, ref) => (
  <SelectPrimitive.Trigger
    ref={ref}
    className={cn(
      // Drawn as a text field (see Input): 32px, one faint 1px border and a light fill, no shadows.
      'flex h-8 w-full items-center justify-between gap-2.5 whitespace-nowrap rounded-md border border-field-border bg-field px-3 py-1 text-[13px] font-medium text-field-foreground transition-[border-color,box-shadow] data-[placeholder]:text-field-placeholder focus:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8 disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
      className
    )}
    {...props}
  >
    {children}
    <SelectPrimitive.Icon asChild>
      <SelectChevrons aria-hidden className="shrink-0" />
    </SelectPrimitive.Icon>
  </SelectPrimitive.Trigger>
))
SelectTrigger.displayName = SelectPrimitive.Trigger.displayName

const SelectScrollUpButton = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.ScrollUpButton>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.ScrollUpButton>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.ScrollUpButton
    ref={ref}
    className={cn('flex cursor-default items-center justify-center py-1', className)}
    {...props}
  >
    <ChevronUpIcon />
  </SelectPrimitive.ScrollUpButton>
))
SelectScrollUpButton.displayName = SelectPrimitive.ScrollUpButton.displayName

const SelectScrollDownButton = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.ScrollDownButton>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.ScrollDownButton>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.ScrollDownButton
    ref={ref}
    className={cn('flex cursor-default items-center justify-center py-1', className)}
    {...props}
  >
    <ChevronDownIcon />
  </SelectPrimitive.ScrollDownButton>
))
SelectScrollDownButton.displayName = SelectPrimitive.ScrollDownButton.displayName

const SelectContent = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content>
>(({ className, children, position = 'popper', collisionPadding = 12, ...props }, ref) => (
  <SelectPrimitive.Portal>
    <SelectPrimitive.Content
      ref={ref}
      // Kept 12px off the window's edges when it has to shift to fit.
      collisionPadding={collisionPadding}
      className={cn(
        'mt-dropdown relative z-50 max-h-96 min-w-[8rem] overflow-hidden',
        menuSurface,
        position === 'popper' &&
          'data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1',
        className
      )}
      position={position}
      {...props}
    >
      <SelectScrollUpButton />
      <SelectPrimitive.Viewport
        className={cn(
          'p-1',
          position === 'popper' &&
            'h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)]'
        )}
      >
        {children}
      </SelectPrimitive.Viewport>
      <SelectScrollDownButton />
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>
))
SelectContent.displayName = SelectPrimitive.Content.displayName

const SelectLabel = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Label ref={ref} className={cn(menuLabel, className)} {...props} />
))
SelectLabel.displayName = SelectPrimitive.Label.displayName

const SelectItem = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item> & { badge?: React.ReactNode }
>(({ className, children, badge, ...props }, ref) => (
  <SelectPrimitive.Item
    ref={ref}
    className={cn(
      'relative flex w-full select-none items-center pl-1.5 pr-8 outline-none focus:before:opacity-100 focus:before:scale-100 data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
      menuItem,
      className
    )}
    {...props}
  >
    <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
      <SelectPrimitive.ItemIndicator>
        <CheckIcon className="h-4 w-4 text-menu-foreground" />
      </SelectPrimitive.ItemIndicator>
    </span>
    {badge ? (
      <div className="flex w-full justify-between">
        <div>
          <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
        </div>
        {badge}
      </div>
    ) : (
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    )}
  </SelectPrimitive.Item>
))
SelectItem.displayName = SelectPrimitive.Item.displayName

const SelectSeparator = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Separator>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Separator ref={ref} className={cn(menuSeparator, className)} {...props} />
))
SelectSeparator.displayName = SelectPrimitive.Separator.displayName

interface SelectEmptyProps {
  children?: React.ReactNode
}
const SelectEmpty = (props: SelectEmptyProps) => {
  return (
    <SelectGroup>
      <SelectLabel>{props.children ?? 'No data'}</SelectLabel>
    </SelectGroup>
  )
}
SelectEmpty.displayName = 'SelectEmpty'

export {
  Select,
  SelectContent,
  SelectEmpty,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
}
