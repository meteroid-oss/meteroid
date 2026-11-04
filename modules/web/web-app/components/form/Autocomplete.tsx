import { Badge, cn, Input } from '@md/ui'
import { XIcon } from 'lucide-react'
import { ReactNode, useMemo, useRef, useState } from 'react'

export interface AutocompleteItem {
  /** The committed string value of this suggestion. */
  value: string
  /** Custom row content; defaults to the value. */
  node?: ReactNode
  /** Extra searchable text (e.g. sample values) for client-side filtering. */
  keywords?: string[]
}

const matches = (item: AutocompleteItem, q: string) => {
  const needle = q.trim().toLowerCase()
  if (!needle) return true
  return (
    item.value.toLowerCase().includes(needle) ||
    !!item.keywords?.some(k => k.toLowerCase().includes(needle))
  )
}

/** The floating suggestion list. `onMouseDown preventDefault` keeps the input
 *  focused so a click selects rather than blurs-then-closes. */
const SuggestionList = ({
  open,
  items,
  active,
  isLoading,
  emptyText,
  footer,
  onHover,
  onPick,
}: {
  open: boolean
  items: AutocompleteItem[]
  active: number
  isLoading?: boolean
  emptyText: string
  footer?: ReactNode
  onHover: (i: number) => void
  onPick: (item: AutocompleteItem) => void
}) => {
  if (!open) return null
  return (
    <div
      onMouseDown={e => e.preventDefault()}
      className="absolute z-50 top-full mt-1 w-full min-w-[220px] rounded-md border border-border bg-popover text-popover-foreground shadow-md"
    >
      <div className="max-h-[240px] overflow-y-auto p-1">
        {isLoading && items.length === 0 ? (
          <div className="px-2 py-3 text-xs text-muted-foreground">Searching…</div>
        ) : items.length === 0 ? (
          <div className="px-2 py-3 text-xs text-muted-foreground">{emptyText}</div>
        ) : (
          items.map((it, i) => (
            <button
              type="button"
              key={it.value}
              onMouseEnter={() => onHover(i)}
              onClick={() => onPick(it)}
              className={cn(
                'flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs',
                i === active ? 'bg-accent text-accent-foreground' : 'text-foreground'
              )}
            >
              {it.node ?? <span className="truncate">{it.value}</span>}
            </button>
          ))
        )}
      </div>
      {footer && (
        <div className="border-t border-border px-2 py-1.5 text-[10px] text-muted-foreground">
          {footer}
        </div>
      )}
    </div>
  )
}

export interface AutocompleteProps {
  value: string
  onValueChange: (v: string) => void
  items: AutocompleteItem[]
  /** Called with the current text so callers can drive server-side search. */
  onQueryChange?: (q: string) => void
  isLoading?: boolean
  placeholder?: string
  emptyText?: string
  footer?: ReactNode
  disabled?: boolean
  className?: string
}

/**
 * A single-value inline typeahead: a normal-looking text input whose suggestions
 * float below as you type. Free text is always allowed — the field value is just
 * the input text — so a not-yet-seen key/code can still be entered. Suggestions are
 * filtered client-side by the current text; pass `onQueryChange` to also drive a
 * server search.
 */
export const Autocomplete = ({
  value,
  onValueChange,
  items,
  onQueryChange,
  isLoading,
  placeholder,
  emptyText = 'No matches',
  footer,
  disabled,
  className,
}: AutocompleteProps) => {
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(-1)
  const filtered = useMemo(() => items.filter(it => matches(it, value)), [items, value])
  const close = () => {
    setOpen(false)
    setActive(-1)
  }
  const pick = (item: AutocompleteItem) => {
    onValueChange(item.value)
    close()
  }

  return (
    <div className={cn('relative', className)}>
      <Input
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        autoComplete="off"
        onChange={e => {
          onValueChange(e.target.value)
          onQueryChange?.(e.target.value)
          setOpen(true)
          setActive(-1)
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => window.setTimeout(close, 120)}
        onKeyDown={e => {
          if (e.key === 'ArrowDown') {
            e.preventDefault()
            setOpen(true)
            setActive(a => Math.min(a + 1, filtered.length - 1))
          } else if (e.key === 'ArrowUp') {
            e.preventDefault()
            setActive(a => Math.max(a - 1, 0))
          } else if (e.key === 'Enter' && open && active >= 0 && filtered[active]) {
            e.preventDefault()
            pick(filtered[active])
          } else if (e.key === 'Escape') {
            close()
          }
        }}
      />
      <SuggestionList
        open={open}
        items={filtered}
        active={active}
        isLoading={isLoading}
        emptyText={emptyText}
        footer={footer}
        onHover={setActive}
        onPick={pick}
      />
    </div>
  )
}

export interface AutocompleteTagsProps {
  value: string[] | undefined
  onChange: (v: string[]) => void
  items: AutocompleteItem[]
  onQueryChange?: (q: string) => void
  isLoading?: boolean
  placeholder?: string
  emptyText?: string
}

/**
 * A multi-value tag input with the same typeahead: existing values render as
 * removable chips, and the in-progress input suggests observed values (excluding
 * ones already chosen). Enter/comma or picking a suggestion adds a tag; free text
 * is allowed.
 */
export const AutocompleteTags = ({
  value = [],
  onChange,
  items,
  onQueryChange,
  isLoading,
  placeholder = 'Type and press Enter',
  emptyText = 'No matches',
}: AutocompleteTagsProps) => {
  const [input, setInput] = useState('')
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(-1)
  const inputRef = useRef<HTMLInputElement>(null)

  const suggestions = useMemo(
    () => items.filter(it => !value.includes(it.value) && matches(it, input)),
    [items, value, input]
  )

  const close = () => {
    setOpen(false)
    setActive(-1)
  }
  const add = (val: string) => {
    const trimmed = val.trim()
    if (trimmed && !value.includes(trimmed)) onChange([...value, trimmed])
    setInput('')
    onQueryChange?.('')
    close()
  }
  const remove = (i: number) => onChange(value.filter((_, idx) => idx !== i))

  return (
    <div className="relative">
      <div
        className="flex min-h-9 flex-wrap items-center gap-1 rounded-md border border-border bg-input px-2 py-1 cursor-text"
        onClick={() => inputRef.current?.focus()}
      >
        {value.map((val, i) => (
          <Badge key={i} variant="secondary" className="gap-1 pr-1">
            {val}
            <button
              type="button"
              onClick={e => {
                e.stopPropagation()
                remove(i)
              }}
              className="rounded-sm hover:bg-muted"
            >
              <XIcon className="h-3 w-3" />
            </button>
          </Badge>
        ))}
        <input
          ref={inputRef}
          type="text"
          value={input}
          autoComplete="off"
          placeholder={value.length === 0 ? placeholder : ''}
          className="min-w-[80px] flex-1 bg-transparent py-0.5 text-sm outline-none"
          onChange={e => {
            setInput(e.target.value)
            onQueryChange?.(e.target.value)
            setOpen(true)
            setActive(-1)
          }}
          onFocus={() => setOpen(true)}
          onBlur={() =>
            window.setTimeout(() => {
              if (input.trim()) add(input)
              else close()
            }, 120)
          }
          onKeyDown={e => {
            if (e.key === 'Enter' || e.key === ',') {
              e.preventDefault()
              if (open && active >= 0 && suggestions[active]) add(suggestions[active].value)
              else add(input)
            } else if (e.key === 'ArrowDown') {
              e.preventDefault()
              setOpen(true)
              setActive(a => Math.min(a + 1, suggestions.length - 1))
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              setActive(a => Math.max(a - 1, 0))
            } else if (e.key === 'Backspace' && input === '' && value.length > 0) {
              remove(value.length - 1)
            } else if (e.key === 'Escape') {
              close()
            }
          }}
        />
      </div>
      <SuggestionList
        open={open}
        items={suggestions}
        active={active}
        isLoading={isLoading}
        emptyText={emptyText}
        onHover={setActive}
        onPick={it => {
          add(it.value)
          inputRef.current?.focus()
        }}
      />
    </div>
  )
}
