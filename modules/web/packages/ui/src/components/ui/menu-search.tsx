import { MagnifyingGlassIcon } from '@radix-ui/react-icons'
import * as React from 'react'

import { cn } from '@ui/lib'

// Keys the menu keeps: moving to the items, choosing one, closing. Every other key is typing.
const MENU_KEYS = new Set(['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'])

// Just after opening, the menu moves focus to itself or its selected item; the field takes it back.
const OPEN_FOCUS_WINDOW_MS = 300

/**
 * The search row at the top of a menu (a DropdownMenu's content): part of the menu rather than
 * a field inside it, focused when the menu opens so typing filters at once, and sticking to the
 * top while the items scroll under it. Typing stays in the field, where the menu's own
 * type-to-select would otherwise take the keys.
 *
 * For a list that searches as you type, prefer a Command in a Popover (cmdk keeps focus in its
 * input and moves through the items without taking it).
 */
const MenuSearch = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, onKeyDown, onBlur, autoFocus = true, ...props }, forwardedRef) => {
    const inputRef = React.useRef<HTMLInputElement>(null)
    React.useImperativeHandle(forwardedRef, () => inputRef.current as HTMLInputElement)
    const openedAt = React.useRef(0)
    React.useEffect(() => {
      openedAt.current = Date.now()
      if (autoFocus) inputRef.current?.focus({ preventScroll: true })
    }, [autoFocus])

    return (
      <div className="sticky top-0 z-10 -mx-1 -mt-1 mb-1 flex h-9 items-center gap-2 border-b border-foreground/12 bg-menu px-3">
        <MagnifyingGlassIcon className="h-3.5 w-3.5 shrink-0 text-menu-foreground/50" aria-hidden />
        <input
          ref={inputRef}
          type="text"
          className={cn(
            'h-full w-full min-w-0 bg-transparent text-[13px] text-menu-foreground placeholder:text-menu-foreground/45 outline-none',
            className
          )}
          onKeyDown={event => {
            if (!MENU_KEYS.has(event.key)) event.stopPropagation()
            onKeyDown?.(event)
          }}
          onBlur={event => {
            onBlur?.(event)
            const menu = event.currentTarget.closest('[role=listbox], [role=menu]')
            const to = event.relatedTarget as Node | null
            if (
              autoFocus &&
              Date.now() - openedAt.current < OPEN_FOCUS_WINDOW_MS &&
              (!to || menu?.contains(to))
            ) {
              const input = event.currentTarget
              requestAnimationFrame(() => input.focus({ preventScroll: true }))
            }
          }}
          {...props}
        />
      </div>
    )
  }
)
MenuSearch.displayName = 'MenuSearch'

export { MenuSearch }
