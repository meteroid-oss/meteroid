import { json } from '@codemirror/lang-json'
import {
  Button,
  Input,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  cn,
} from '@md/ui'
import { githubDark, githubLight } from '@uiw/codemirror-theme-github'
import CodeMirror from '@uiw/react-codemirror'
import { ArrowLeftIcon, CheckIcon, CopyIcon } from 'lucide-react'
import { useMemo, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { type WebhookEventType } from '@/rpc/api/webhooksout/v1/models_pb'
import { listWebhookEventTypes } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

/** Every event an endpoint can subscribe to, with what it means and its payload. */
export const EventCatalogPage = ({ onBack }: { onBack: () => void }) => {
  const eventTypesQuery = useQuery(listWebhookEventTypes)
  const [selectedName, setSelectedName] = useState<string | null>(null)
  const [search, setSearch] = useState('')

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    const byGroup = new Map<string, WebhookEventType[]>()
    for (const eventType of eventTypesQuery.data?.eventTypes ?? []) {
      if (needle && !eventType.name.includes(needle)) continue
      const existing = byGroup.get(eventType.group)
      if (existing) existing.push(eventType)
      else byGroup.set(eventType.group, [eventType])
    }
    return [...byGroup.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [eventTypesQuery.data?.eventTypes, search])

  const selected =
    eventTypesQuery.data?.eventTypes.find(e => e.name === selectedName) ?? groups[0]?.[1][0]

  return (
    <div className="flex max-w-5xl flex-col gap-5">
      <div className="space-y-3">
        <Button variant="ghost" size="sm" hasIcon className="-ml-2" onClick={onBack}>
          <ArrowLeftIcon size={14} /> Webhooks
        </Button>
        <div className="space-y-1">
          <h3 className="text-base font-semibold">Event catalog</h3>
          <p className="text-sm text-muted-foreground">
            The events you can subscribe an endpoint to. Amounts are integers in the currency&apos;s
            minor unit, so 129000 in EUR is €1,290.00.
          </p>
        </div>
      </div>

      {eventTypesQuery.isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : (
        <div className="flex min-h-0 gap-6">
          <nav className="w-60 shrink-0 space-y-4">
            <Input
              className="h-8"
              placeholder="Search events"
              value={search}
              onChange={e => setSearch(e.target.value)}
            />
            {groups.map(([group, eventTypes]) => (
              <div key={group} className="space-y-1">
                <div className="flex items-center justify-between px-2 text-xs font-medium capitalize text-muted-foreground">
                  <span>{group}</span>
                  <span>{eventTypes.length}</span>
                </div>
                {eventTypes.map(eventType => (
                  <button
                    key={eventType.name}
                    type="button"
                    onClick={() => setSelectedName(eventType.name)}
                    className={cn(
                      'block w-full truncate rounded-md px-2 py-1 text-left font-mono text-xs hover:bg-accent/40',
                      selected?.name === eventType.name && 'bg-accent text-accent-foreground'
                    )}
                  >
                    {eventType.name}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          {selected && <EventTypeDetail eventType={selected} />}
        </div>
      )}
    </div>
  )
}

const EventTypeDetail = ({ eventType }: { eventType: WebhookEventType }) => {
  const { isDarkMode } = useTheme()
  const [copied, setCopied] = useState(false)

  return (
    <div className="min-w-0 flex-1 space-y-5">
      <div className="space-y-1">
        <h3 className="font-mono text-sm font-medium">{eventType.name}</h3>
        {eventType.description && (
          <p className="text-sm text-muted-foreground">{eventType.description}</p>
        )}
      </div>

      <section className="space-y-1.5">
        <h4 className="text-xs font-medium text-muted-foreground">Fields</h4>
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Field</TableHead>
                <TableHead>Type</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {eventType.fields.map(field => (
                <TableRow key={field.path}>
                  <TableCell className="align-top">
                    <div className="font-mono text-xs">{field.path}</div>
                    {field.description && (
                      <div className="text-xs text-muted-foreground">{field.description}</div>
                    )}
                  </TableCell>
                  <TableCell className="align-top text-xs text-muted-foreground">
                    {field.type}
                    {!field.required && <span> · optional</span>}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </section>

      <section className="space-y-1.5">
        <div className="flex items-center justify-between">
          <h4 className="text-xs font-medium text-muted-foreground">Example</h4>
          <Button
            variant="ghost"
            size="sm"
            hasIcon
            onClick={() =>
              copyToClipboard(eventType.exampleJson, () => {
                setCopied(true)
                setTimeout(() => setCopied(false), 1200)
              })
            }
          >
            {copied ? <CheckIcon size={14} /> : <CopyIcon size={14} />} Copy
          </Button>
        </div>
        <div className="overflow-hidden rounded-lg border border-border">
          <CodeMirror
            value={eventType.exampleJson}
            extensions={[json()]}
            theme={isDarkMode ? githubDark : githubLight}
            editable={false}
            readOnly
            basicSetup={{ foldGutter: false, highlightActiveLine: false }}
          />
        </div>
      </section>
    </div>
  )
}
