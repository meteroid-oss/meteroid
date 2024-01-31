import * as RadixPopover from '@radix-ui/react-popover'
import { X as IconX } from 'lucide-react'
import * as React from 'react'

import { cn } from '@ui/lib/cn'

import { twPopoverStyles } from './PopoverAlt.styles'

import type * as RadixPopoverTypes from '@radix-ui/react-popover/'

interface RootProps {
  align?: RadixPopoverTypes.PopoverContentProps['align']
  ariaLabel?: string
  arrow?: boolean
  children?: React.ReactNode
  className?: string
  triggerClassName?: string
  defaultOpen?: boolean
  modal?: boolean
  onOpenChange?: RadixPopoverTypes.PopoverProps['onOpenChange']
  open?: boolean
  overlay?: React.ReactNode
  showClose?: boolean
  side?: RadixPopoverTypes.PopoverContentProps['side']
  sideOffset?: RadixPopoverTypes.PopoverContentProps['sideOffset']
  style?: React.CSSProperties
  header?: React.ReactNode
  footer?: React.ReactNode
  size?: 'tiny' | 'small' | 'medium' | 'large' | 'xlarge' | 'content'
}

function PopoverAlt({
  align = 'center',
  ariaLabel,
  arrow = false,
  children,
  className,
  triggerClassName,
  defaultOpen = false,
  modal,
  onOpenChange,
  open,
  overlay,
  side = 'bottom',
  sideOffset = 6,
  style,
  header,
  footer,
  size = 'content',
}: RootProps) {
  const __styles = twPopoverStyles.popover

  const classes = [__styles.content, __styles.size[size]]
  if (className) {
    classes.push(className)
  }

  const triggerClasses = [__styles.trigger]
  if (triggerClassName) {
    triggerClasses.push(triggerClassName)
  }
  return (
    <RadixPopover.Root
      defaultOpen={defaultOpen}
      modal={modal}
      onOpenChange={onOpenChange}
      open={open}
    >
      <RadixPopover.Trigger className={cn(triggerClasses)} aria-label={ariaLabel}>
        {children}
      </RadixPopover.Trigger>

      <RadixPopover.Portal>
        <RadixPopover.Content
          sideOffset={sideOffset}
          side={side}
          align={align}
          className={cn(classes)}
          style={style}
        >
          {arrow && <RadixPopover.Arrow offset={10}></RadixPopover.Arrow>}
          {header && <div className={cn(__styles.header)}>{header}</div>}
          {overlay}
          {footer && <div className={cn(__styles.footer)}>{footer}</div>}
        </RadixPopover.Content>
      </RadixPopover.Portal>
    </RadixPopover.Root>
  )
}

function Close() {
  const __styles = twPopoverStyles.popover

  return (
    <RadixPopover.Close className={cn(__styles.close)}>
      <IconX size={14} strokeWidth={2} />
    </RadixPopover.Close>
  )
}

function Separator() {
  const __styles = twPopoverStyles.popover

  return <div className={__styles.separator}></div>
}

PopoverAlt.Separator = Separator
PopoverAlt.Close = Close
export default PopoverAlt
