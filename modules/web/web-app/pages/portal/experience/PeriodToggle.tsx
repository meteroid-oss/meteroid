import { useRef } from 'react'

import type { OptionChoice } from './variants'
import type { KeyboardEvent } from 'react'

/** Segmented "Monthly | Annual" switch of the plans' options; a radio group with arrow keys. */
export const PeriodToggle = ({
  choices,
  value,
  onChange,
}: {
  choices: OptionChoice[]
  value: string | undefined
  onChange: (alias: string) => void
}) => {
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  const activeIndex = Math.max(0, choices.findIndex(c => c.alias === value))

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step =
      e.key === 'ArrowRight' || e.key === 'ArrowDown'
        ? 1
        : e.key === 'ArrowLeft' || e.key === 'ArrowUp'
          ? -1
          : 0
    if (!step) return
    e.preventDefault()
    const next = (activeIndex + step + choices.length) % choices.length
    onChange(choices[next].alias)
    refs.current[next]?.focus()
  }

  return (
    <div
      role="radiogroup"
      aria-label="Option"
      onKeyDown={onKeyDown}
      style={{
        display: 'inline-flex',
        gap: 2,
        padding: 3,
        borderRadius: 'var(--mtp-r-ctrl)',
        background: 'var(--mtp-track)',
        border: '1px solid var(--mtp-border)',
      }}
    >
      {choices.map((choice, i) => {
        const checked = i === activeIndex
        return (
          <button
            key={choice.alias}
            ref={el => {
              refs.current[i] = el
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            className="mtp-btn"
            onClick={() => onChange(choice.alias)}
            style={{
              padding: '5px 14px',
              borderRadius: 'var(--mtp-r-sm)',
              border: 'none',
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: 12.5,
              fontWeight: checked ? 600 : 500,
              background: checked ? 'var(--mtp-surface)' : 'transparent',
              color: checked ? 'var(--mtp-text)' : 'var(--mtp-text-2)',
              boxShadow: checked ? '0 1px 2px rgba(0,0,0,0.08)' : 'none',
            }}
          >
            {choice.name}
          </button>
        )
      })}
    </div>
  )
}
