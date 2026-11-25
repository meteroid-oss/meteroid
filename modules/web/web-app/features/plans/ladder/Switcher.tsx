import { cn } from '@md/ui'
import { KeyboardEvent, ReactNode, useRef } from 'react'

export interface SwitcherItem<T extends string> {
  value: T
  label: ReactNode
}

/** A segmented switch: one tab stop, arrow keys move and select. */
export function Switcher<T extends string>({
  value,
  items,
  onChange,
  label,
  size = 'md',
  className,
}: {
  value: T | undefined
  items: SwitcherItem<T>[]
  onChange: (value: T) => void
  label: string
  size?: 'sm' | 'md'
  className?: string
}) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([])
  const current = Math.max(0, items.findIndex(item => item.value === value))

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step =
      event.key === 'ArrowRight' || event.key === 'ArrowDown'
        ? 1
        : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
          ? -1
          : 0
    if (step === 0) return
    event.preventDefault()
    const target = (current + step + items.length) % items.length
    buttons.current[target]?.focus()
    onChange(items[target].value)
  }

  return (
    <div
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className={cn(
        'inline-flex items-center gap-0.5 rounded-lg border border-border bg-card p-1',
        className
      )}
    >
      {items.map((item, index) => {
        const checked = item.value === value
        return (
          <button
            key={item.value}
            ref={element => {
              buttons.current[index] = element
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={index === current ? 0 : -1}
            onClick={() => !checked && onChange(item.value)}
            className={cn(
              'whitespace-nowrap rounded-md font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              size === 'md' ? 'px-3 py-1 text-sm' : 'px-2.5 py-1 text-xs',
              checked
                ? 'bg-accent text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            {item.label}
          </button>
        )
      })}
    </div>
  )
}
