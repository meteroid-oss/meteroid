import * as RadixDropdown from '@radix-ui/react-dropdown-menu'
import { Check as IconCheck, Target as IconTarget } from 'lucide-react'
import { useState } from 'react'

import { cn } from '@ui/lib/cn'

import { twDropdownStyles } from './Dropdown.styles'

import type * as RadixDropdownTypes from '@radix-ui/react-dropdown-menu/'

interface RootProps extends RadixDropdownTypes.DropdownMenuProps {
  open?: boolean
  arrow?: boolean
  side?: RadixDropdownTypes.DropdownMenuContentProps['side']
  align?: RadixDropdownTypes.DropdownMenuContentProps['align']
  sideOffset?: RadixDropdownTypes.DropdownMenuContentProps['sideOffset']
  overlay?: RadixDropdownTypes.DropdownMenuContentProps['children']
  children?: RadixDropdownTypes.DropdownMenuTriggerProps['children']
  size?: 'tiny' | 'small' | 'medium' | 'large' | 'xlarge' | 'content'
  className?: string
  style?: React.CSSProperties
  isNested?: boolean
}

function Dropdown({
  align = 'center',
  side = 'bottom',
  sideOffset = 6,
  overlay,
  children,
  size = 'medium',
  className,
  style,
  arrow,
  isNested,
  ...props
}: RootProps) {
  const __styles = twDropdownStyles.dropdown

  const classes = [__styles.content, __styles.size[size]]
  if (className) {
    classes.push(className)
  }

  return (
    <RadixDropdown.Root {...props}>
      {isNested ? (
        <RadixDropdown.Sub>
          <RadixDropdown.SubTrigger className={cn([__styles.item_nested])}>
            {children}
          </RadixDropdown.SubTrigger>
          <RadixDropdown.SubContent sideOffset={sideOffset} className={cn(classes)} style={style}>
            {arrow && (
              <RadixDropdown.Arrow className={cn(__styles.arrow)} offset={10}></RadixDropdown.Arrow>
            )}
            {overlay}
          </RadixDropdown.SubContent>
        </RadixDropdown.Sub>
      ) : (
        <>
          <RadixDropdown.Trigger className={cn(__styles.trigger)}>{children}</RadixDropdown.Trigger>
          <RadixDropdown.Portal>
            <RadixDropdown.Content
              sideOffset={sideOffset}
              side={side}
              align={align}
              className={cn(classes)}
              style={style}
            >
              {arrow && (
                <RadixDropdown.Arrow
                  className={cn(__styles.arrow)}
                  offset={10}
                ></RadixDropdown.Arrow>
              )}
              {overlay}
            </RadixDropdown.Content>
          </RadixDropdown.Portal>
        </>
      )}
    </RadixDropdown.Root>
  )
}

export function RightSlot({ children }: { children: React.ReactNode }) {
  const __styles = twDropdownStyles.dropdown

  return <div className={cn(__styles.right_slot)}>{children}</div>
}

interface ItemProps extends RadixDropdownTypes.DropdownMenuItemProps {
  children: React.ReactNode
  icon?: React.ReactNode
}

export function Item({ icon, className, ...props }: ItemProps) {
  const __styles = twDropdownStyles.dropdown

  return (
    <RadixDropdown.Item
      className={cn(__styles.item, className, props.disabled && __styles.disabled)}
      {...props}
    >
      {icon && icon}
      <span>{props.children}</span>
    </RadixDropdown.Item>
  )
}

export function TriggerItem({ children, icon }: ItemProps) {
  const __styles = twDropdownStyles.dropdown

  return (
    <div className={cn(__styles.item)}>
      {icon && icon}
      <span>{children}</span>
    </div>
  )
}

export function Misc({ children, icon }: ItemProps) {
  const __styles = twDropdownStyles.dropdown

  return (
    <div className={cn(__styles.misc)}>
      {icon && icon}
      {children}
    </div>
  )
}

export function Separator() {
  const __styles = twDropdownStyles.dropdown

  return <RadixDropdown.Separator className={__styles.separator} />
}

// to do  : remove onChange omit in favor of using onCheckedChange
interface CheckboxProps extends Omit<RadixDropdownTypes.DropdownMenuCheckboxItemProps, 'onChange'> {
  ItemIndicator?: React.ReactNode
  onChange?(x: boolean): void
}

export function Checkbox({
  checked: propsChecked,
  ItemIndicator,
  onChange,
  className,
  ...props
}: CheckboxProps) {
  const [checked, setChecked] = useState(propsChecked ?? false)
  const __styles = twDropdownStyles.dropdown

  const handleChange = (e: boolean) => {
    // to do  : remove onChange in favor of using onCheckedChange
    if (onChange) onChange(e)
    if (props.onCheckedChange) props.onCheckedChange(e)
    setChecked(e)
  }

  return (
    <RadixDropdown.CheckboxItem
      checked={checked}
      onCheckedChange={handleChange}
      className={cn(__styles.item, __styles.input, className)}
      {...props}
    >
      <RadixDropdown.ItemIndicator className={__styles.check}>
        {ItemIndicator ? ItemIndicator : <IconCheck size="tiny" strokeWidth={3} />}
        <RadixDropdown.CheckboxItem />
      </RadixDropdown.ItemIndicator>
      <span>{props.children}</span>
    </RadixDropdown.CheckboxItem>
  )
}

interface RadioProps extends RadixDropdownTypes.DropdownMenuRadioItemProps {
  ItemIndicator?: React.ReactNode
}

export function Radio({ ItemIndicator, className, ...props }: RadioProps) {
  const __styles = twDropdownStyles.dropdown

  return (
    <RadixDropdown.RadioItem className={cn(__styles.item, __styles.input, className)} {...props}>
      <RadixDropdown.ItemIndicator className={__styles.check}>
        {ItemIndicator ? ItemIndicator : <IconTarget strokeWidth={6} size={10} />}
      </RadixDropdown.ItemIndicator>
      <span>{props.children}</span>
    </RadixDropdown.RadioItem>
  )
}

// to do  : remove onChange omit in favor of using onValueChange
interface RadioGroupProps extends Omit<RadixDropdownTypes.DropdownMenuRadioGroupProps, 'onChange'> {
  onChange?(x: string): void
}

export function RadioGroup({ value: propsValue, onChange, ...props }: RadioGroupProps) {
  const [value, setValue] = useState(propsValue ? propsValue : '')

  const handleChange = (e: string) => {
    // to do  : remove onChange in favor of using onValueChange
    if (onChange) onChange(e)
    if (props.onValueChange) props.onValueChange(e)
    setValue(e)
  }

  return <RadixDropdown.RadioGroup value={value} onValueChange={handleChange} {...props} />
}

type LabelProps = RadixDropdownTypes.DropdownMenuLabelProps

export function Label({ className, ...props }: LabelProps) {
  const __styles = twDropdownStyles.dropdown

  return <RadixDropdown.Label className={cn(__styles.label, className)} {...props} />
}

Dropdown.Item = Item
Dropdown.Misc = Misc
Dropdown.Checkbox = Checkbox
Dropdown.Radio = Radio
Dropdown.RadioGroup = RadioGroup
Dropdown.Label = Label
Dropdown.Separator = Separator
Dropdown.RightSlot = RightSlot
Dropdown.TriggerItem = TriggerItem
export default Dropdown
