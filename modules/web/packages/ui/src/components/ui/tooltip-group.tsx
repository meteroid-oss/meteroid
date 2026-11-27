import { Slot } from '@radix-ui/react-slot'
import * as React from 'react'
import { createPortal } from 'react-dom'

import { cn } from '@ui/lib'

/**
 * A row of icon buttons that share one tooltip. Moving from button to button makes the tooltip
 * travel to the new button (and resize to its text) instead of hiding and showing again.
 *
 *   <TooltipGroup className="gap-1">
 *     <TooltipGroupItem label="Edit">
 *       <button aria-label="Edit">…</button>
 *     </TooltipGroupItem>
 *   </TooltipGroup>
 *
 * The tooltip is not the buttons' accessible name: give each button its own `aria-label`.
 */

interface TooltipGroupContextValue {
  id: string
  show: (trigger: HTMLElement, label: string) => void
  hide: () => void
}

const TooltipGroupContext = React.createContext<TooltipGroupContextValue | null>(null)

/** Space between the tooltip and its trigger, and the least distance it keeps from the window edge. */
const GAP = 8

const TooltipGroup = ({ className, children, ...props }: React.HTMLAttributes<HTMLDivElement>) => {
  const id = React.useId()
  const tipRef = React.useRef<HTMLDivElement>(null)
  const textRef = React.useRef<HTMLSpanElement>(null)
  const visible = React.useRef(false)

  const hide = React.useCallback(() => {
    visible.current = false
    tipRef.current?.setAttribute('data-visible', 'false')
  }, [])

  const show = React.useCallback((trigger: HTMLElement, label: string) => {
    const tip = tipRef.current
    const text = textRef.current
    if (!tip || !text) return

    text.textContent = label
    const style = getComputedStyle(tip)
    const width = text.offsetWidth + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight)
    const rect = trigger.getBoundingClientRect()
    const x = Math.round(
      Math.min(
        Math.max(rect.left + rect.width / 2 - width / 2, GAP),
        window.innerWidth - width - GAP
      )
    )
    const y = Math.round(rect.top - GAP - tip.offsetHeight)

    const place = () => {
      tip.style.width = `${width}px`
      tip.style.setProperty('--tooltip-x', `${x}px`)
      tip.style.setProperty('--tooltip-y', `${y}px`)
    }

    if (visible.current) {
      // Already showing: let it travel to the new trigger.
      place()
    } else {
      // Coming in: put it in place without motion, then play only the appear transition.
      tip.style.transition = 'none'
      place()
      void tip.offsetWidth
      tip.style.transition = ''
    }
    visible.current = true
    tip.setAttribute('data-visible', 'true')
  }, [])

  // A fixed-position tooltip would be left behind by a scroll, so it hides instead.
  React.useEffect(() => {
    window.addEventListener('scroll', hide, true)
    window.addEventListener('resize', hide)
    return () => {
      window.removeEventListener('scroll', hide, true)
      window.removeEventListener('resize', hide)
    }
  }, [hide])

  const context = React.useMemo(() => ({ id, show, hide }), [id, show, hide])

  return (
    <TooltipGroupContext.Provider value={context}>
      <div
        {...props}
        className={cn('inline-flex items-center', className)}
        onPointerLeave={e => {
          props.onPointerLeave?.(e)
          hide()
        }}
        onPointerDown={e => {
          props.onPointerDown?.(e)
          hide()
        }}
        onBlur={e => {
          props.onBlur?.(e)
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) hide()
        }}
        onKeyDown={e => {
          props.onKeyDown?.(e)
          if (e.key === 'Escape') hide()
        }}
      >
        {children}
      </div>
      {typeof document !== 'undefined' &&
        createPortal(
          <div
            ref={tipRef}
            id={id}
            role="tooltip"
            data-visible="false"
            className="mt-tooltip mt-tooltip-travel text-xs"
          >
            <span ref={textRef} />
          </div>,
          document.body
        )}
    </TooltipGroupContext.Provider>
  )
}
TooltipGroup.displayName = 'TooltipGroup'

interface TooltipGroupItemProps {
  /** The tooltip text for this trigger. */
  label: string
  /** One focusable element, usually an icon button. */
  children: React.ReactElement
}

const TooltipGroupItem = ({ label, children }: TooltipGroupItemProps) => {
  const context = React.useContext(TooltipGroupContext)
  if (!context) throw new Error('TooltipGroupItem must be used within a TooltipGroup')
  const { id, show } = context

  return (
    <Slot
      aria-describedby={id}
      onPointerEnter={(e: React.PointerEvent<HTMLElement>) => {
        if (e.pointerType === 'mouse') show(e.currentTarget, label)
      }}
      onFocus={(e: React.FocusEvent<HTMLElement>) => show(e.currentTarget, label)}
    >
      {children}
    </Slot>
  )
}
TooltipGroupItem.displayName = 'TooltipGroupItem'

export { TooltipGroup, TooltipGroupItem }
