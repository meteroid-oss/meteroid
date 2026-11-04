import { BetaBadge, Checkbox, Switch } from '@md/ui'
import { ChevronDownIcon, ChevronRightIcon } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

import { EventTypesSummary } from '@/features/settings/webhooks/badges'
import { useQuery } from '@/lib/connectrpc'
import { type WebhookEventType } from '@/rpc/api/webhooksout/v1/models_pb'
import { listWebhookEventTypes } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'

interface Props {
  allEvents: boolean
  onAllEventsChange: (allEvents: boolean) => void
  value: string[]
  onChange: (eventTypes: string[]) => void
}

/**
 * Event type picker for an endpoint. "All events" maps to an empty `event_types`
 * list on the wire, which also subscribes the endpoint to event types added later,
 * except Beta ones: those are sent only to endpoints that select them.
 * Grouped checkboxes, like the role editor's permission picker.
 */
export const EventTypeSelect = ({ allEvents, onAllEventsChange, value, onChange }: Props) => {
  const eventTypesQuery = useQuery(listWebhookEventTypes)

  const groups = useMemo(() => {
    const byGroup = new Map<string, WebhookEventType[]>()
    for (const eventType of eventTypesQuery.data?.eventTypes ?? []) {
      const existing = byGroup.get(eventType.group)
      if (existing) existing.push(eventType)
      else byGroup.set(eventType.group, [eventType])
    }
    return [...byGroup.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [eventTypesQuery.data?.eventTypes])

  const selected = new Set(value)

  // Groups start open when something in them is picked, or for the first group.
  const [open, setOpen] = useState<Set<string>>(new Set())
  useEffect(() => {
    if (groups.length === 0) return
    setOpen(previous => {
      if (previous.size > 0) return previous
      const withSelection = groups
        .filter(([, types]) => types.some(t => value.includes(t.name)))
        .map(([group]) => group)
      return new Set(withSelection.length > 0 ? withSelection : [groups[0][0]])
    })
  }, [groups, value])
  const toggleOpen = (group: string) =>
    setOpen(previous => {
      const next = new Set(previous)
      if (next.has(group)) next.delete(group)
      else next.add(group)
      return next
    })

  const toggle = (name: string) =>
    onChange(selected.has(name) ? value.filter(v => v !== name) : [...value, name])

  const toggleGroup = (names: string[], on: boolean) => {
    const next = new Set(value)
    names.forEach(name => (on ? next.add(name) : next.delete(name)))
    onChange([...next])
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-4 rounded-md border border-border px-3 py-2">
        <div className="min-w-0">
          <div className="text-[13px] font-medium">All event types</div>
          <div className="text-xs leading-snug text-muted-foreground">
            Every event type except Beta ones, including types added later. To receive a Beta
            event type, turn this off and pick it.
          </div>
        </div>
        <Switch checked={allEvents} onCheckedChange={onAllEventsChange} />
      </div>

      {!allEvents && (
        <div className="overflow-hidden rounded-md border border-border">
          <div className="flex items-center justify-between gap-4 border-b border-border px-3 py-2 text-sm">
            {value.length === 0 ? (
              <span className="text-muted-foreground">No event type selected</span>
            ) : (
              <span>
                <span className="text-muted-foreground">Selected: </span>
                <EventTypesSummary eventTypes={value} />
              </span>
            )}
            {value.length > 0 && (
              <button
                type="button"
                className="text-xs text-muted-foreground hover:text-foreground"
                onClick={() => onChange([])}
              >
                Clear all
              </button>
            )}
          </div>
          <div className="max-h-[300px] space-y-4 overflow-y-auto p-3">
            {eventTypesQuery.isLoading ? (
              <p className="text-sm text-muted-foreground">Loading…</p>
            ) : groups.length === 0 ? (
              <p className="text-sm text-muted-foreground">No event types</p>
            ) : (
              groups.map(([group, eventTypes]) => {
                const names = eventTypes.map(eventType => eventType.name)
                const beta = eventTypes.every(eventType => eventType.beta)
                const checked = names.filter(name => selected.has(name)).length
                const allChecked = checked === names.length
                const isOpen = open.has(group)
                return (
                  <div key={group} className="space-y-1">
                    <div className="flex items-center justify-between">
                      <button
                        type="button"
                        className="flex items-center gap-1.5 text-sm font-medium capitalize"
                        onClick={() => toggleOpen(group)}
                      >
                        {isOpen ? <ChevronDownIcon size={14} /> : <ChevronRightIcon size={14} />}
                        {group}
                        {beta && <BetaBadge />}
                        <span className="text-xs font-normal normal-case text-muted-foreground">
                          {checked} of {names.length}
                        </span>
                      </button>
                      <button
                        type="button"
                        className="text-xs text-muted-foreground hover:text-foreground"
                        onClick={() => toggleGroup(names, !allChecked)}
                      >
                        {allChecked ? 'Clear' : 'Select all'}
                      </button>
                    </div>
                    {isOpen && beta && (
                      <p className="pl-5 text-xs text-muted-foreground">
                        Sent only to endpoints that select these events, never through All event
                        types.
                      </p>
                    )}
                    {isOpen && (
                      <div className="space-y-0.5 pl-5">
                        {eventTypes.map(eventType => (
                          <label
                            key={eventType.name}
                            className="flex cursor-pointer items-start gap-2 rounded-md px-1.5 py-1 hover:bg-accent/40"
                          >
                            <Checkbox
                              className="mt-0.5"
                              checked={selected.has(eventType.name)}
                              onCheckedChange={() => toggle(eventType.name)}
                            />
                            <span className="min-w-0">
                              <span className="block truncate font-mono text-xs">
                                {eventType.name}
                              </span>
                              {eventType.description && (
                                <span className="block text-xs text-muted-foreground">
                                  {eventType.description}
                                </span>
                              )}
                            </span>
                          </label>
                        ))}
                      </div>
                    )}
                  </div>
                )
              })
            )}
          </div>
        </div>
      )}
    </div>
  )
}
