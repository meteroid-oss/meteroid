import { Input, Popover, PopoverAnchor, PopoverContent, cn } from '@md/ui'
import { menuItem } from '@ui/components/ui/menu-styles'
import { Search, X, type LucideIcon } from 'lucide-react'
import { Fragment, useEffect, useMemo, useRef, useState } from 'react'

import { SETTINGS_SEARCH_ENTRIES, type SettingsSearchEntry } from './settingsSearchIndex'

interface SearchableTab {
  value: string
  label: string
  title?: string
  icon: LucideIcon
}

interface SearchableSection {
  title: string
  items: SearchableTab[]
}

interface Result {
  key: string
  tab: SearchableTab
  entry?: SettingsSearchEntry
}

/** The text with each occurrence of the query drawn in the accent colour. */
const Highlighted = ({ text, needle }: { text: string; needle: string }) => {
  const lower = text.toLowerCase()
  const parts: { text: string; hit: boolean }[] = []
  let from = 0
  for (let at = lower.indexOf(needle); needle && at !== -1; at = lower.indexOf(needle, from)) {
    if (at > from) parts.push({ text: text.slice(from, at), hit: false })
    parts.push({ text: text.slice(at, at + needle.length), hit: true })
    from = at + needle.length
  }
  if (from < text.length) parts.push({ text: text.slice(from), hit: false })
  return (
    <>
      {parts.map((part, i) =>
        part.hit ? (
          <span key={i} className="text-brand">
            {part.text}
          </span>
        ) : (
          <Fragment key={i}>{part.text}</Fragment>
        )
      )}
    </>
  )
}

/**
 * The search at the top of the settings nav: a list under the field of the tabs and of the
 * sections, rows and fields inside them that match. Choosing one opens the tab (and, for a
 * section, scrolls to it). Arrow keys move through the list, Enter chooses, Escape clears.
 */
export const SettingsSearch = ({
  sections,
  onSelect,
}: {
  sections: SearchableSection[]
  onSelect: (tab: string, entry?: SettingsSearchEntry) => void
}) => {
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const anchorRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const needle = query.trim().toLowerCase()

  // A tab is listed when it, its section or anything inside it matches; under it, the matches inside.
  const results = useMemo<Result[]>(() => {
    if (!needle) return []
    const matches = (text?: string) => !!text && text.toLowerCase().includes(needle)
    return sections.flatMap(section =>
      section.items.flatMap(tab => {
        const own = tab.label.toLowerCase()
        const hits = (SETTINGS_SEARCH_ENTRIES[tab.value] ?? []).filter(
          entry => entry.text.toLowerCase() !== own && matches(entry.text)
        )
        const tabMatches = matches(tab.label) || matches(tab.title) || matches(section.title)
        if (!tabMatches && hits.length === 0) return []
        return [
          { key: tab.value, tab },
          ...hits.map(entry => ({ key: `${tab.value}/${entry.text}`, tab, entry })),
        ]
      })
    )
  }, [sections, needle])

  useEffect(() => {
    setActive(0)
  }, [needle])
  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${active}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [active])

  const choose = (result: Result) => {
    onSelect(result.tab.value, result.entry)
    setQuery('')
    setOpen(false)
  }

  return (
    <Popover open={open && !!needle} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <div ref={anchorRef} className="relative w-full">
          <Search
            size={14}
            // A solid colour: a translucent one shows where the strokes cross.
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint-foreground"
            aria-hidden
          />
          <Input
            ref={inputRef}
            type="search"
            role="combobox"
            aria-expanded={open && !!needle}
            aria-controls="settings-search-results"
            aria-activedescendant={results[active] ? `settings-search-${active}` : undefined}
            aria-autocomplete="list"
            aria-label="Search settings"
            placeholder="Search settings"
            value={query}
            onChange={event => {
              setQuery(event.target.value)
              setOpen(true)
            }}
            onFocus={() => setOpen(true)}
            onKeyDown={event => {
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                event.preventDefault()
                if (results.length === 0) return
                setOpen(true)
                const step = event.key === 'ArrowDown' ? 1 : -1
                setActive(i => (i + step + results.length) % results.length)
              } else if (event.key === 'Enter' && results[active]) {
                event.preventDefault()
                choose(results[active])
              }
            }}
            className="pl-8 pr-8 [&::-webkit-search-cancel-button]:hidden"
          />
          {query && (
            <button
              type="button"
              onClick={() => {
                setQuery('')
                inputRef.current?.focus()
              }}
              className="absolute right-1.5 top-1/2 grid size-5 -translate-y-1/2 place-items-center rounded text-faint-foreground hover:text-foreground"
              aria-label="Clear search"
            >
              <X size={12} />
            </button>
          )}
        </div>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        sideOffset={6}
        // Typing stays in the field: the list never takes focus.
        onOpenAutoFocus={event => event.preventDefault()}
        onCloseAutoFocus={event => event.preventDefault()}
        onInteractOutside={event => {
          if (anchorRef.current?.contains(event.target as Node)) event.preventDefault()
        }}
        onEscapeKeyDown={() => setQuery('')}
        className="w-[300px] max-h-[min(420px,var(--radix-popover-content-available-height))] overflow-y-auto p-1"
      >
        <div ref={listRef} id="settings-search-results" role="listbox" aria-label="Settings">
          {results.length === 0 ? (
            <p className="px-2 py-1.5 text-[13px] text-menu-foreground/55">
              No settings match “{query.trim()}”
            </p>
          ) : (
            results.map((result, index) => (
              <div
                key={result.key}
                id={`settings-search-${index}`}
                data-index={index}
                role="option"
                aria-selected={index === active}
                onMouseDown={event => event.preventDefault()}
                onMouseMove={() => setActive(index)}
                onClick={() => choose(result)}
                className={cn(
                  menuItem,
                  'flex items-center px-2',
                  index === active && 'before:opacity-100 before:scale-100'
                )}
              >
                {result.entry ? (
                  // Inside a tab: aligned with the tab's label, under its icon.
                  <span className="truncate pl-6">
                    <Highlighted text={result.entry.text} needle={needle} />
                  </span>
                ) : (
                  <>
                    <result.tab.icon aria-hidden />
                    <span className="truncate">
                      <Highlighted text={result.tab.label} needle={needle} />
                    </span>
                  </>
                )}
              </div>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}
