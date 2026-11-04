import { timestampDate } from '@bufbuild/protobuf/wkt'
import {
  Badge,
  Card,
  CardContent,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Label,
} from '@md/ui'
import { Link } from 'react-router-dom'

import { PropertyProfilePanel } from '@/features/events/PropertyProfilePanel'
import { useBasePath } from '@/hooks/useBasePath'

import type { EventSummary } from '@/rpc/api/events/v1/events_pb'

/**
 * Stringify event properties with keys sorted, so the same property set renders in a
 * stable order across rows — the underlying proto `map` has no guaranteed key order.
 */
export const stringifyProperties = (props: Record<string, string>, indent?: number) =>
  JSON.stringify(props, Object.keys(props).sort(), indent)

/**
 * An event's customer as a link to the customer detail page, resolved server-side
 * to the internal id + display name. Falls back to the raw identity string (an
 * external alias or an unresolved id) as plain text when resolution failed.
 */
export const EventCustomerLink = ({ event }: { event: EventSummary }) => {
  const basePath = useBasePath()

  if (event.customerLocalId) {
    return (
      <Link
        to={`${basePath}/customers/${event.customerLocalId}`}
        className="font-mono text-xs text-brand hover:underline"
      >
        {event.customerName || event.customerId}
      </Link>
    )
  }

  return <span className="font-mono text-xs">{event.customerId || '—'}</span>
}

/** Full event inspector (the "eye" affordance), shared by the Events page and the
 * billable-metric preview's raw-events tab. */
export const EventDetailsDialog = ({
  event,
  onClose,
}: {
  event: EventSummary | null
  onClose: () => void
}) => (
  <Dialog open={!!event} onOpenChange={open => !open && onClose()}>
    <DialogContent className="sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle>Event Details</DialogTitle>
        <DialogDescription>Full event information and properties</DialogDescription>
      </DialogHeader>
      {event && (
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>Event ID</Label>
              <div className="font-mono text-sm">{event.id}</div>
            </div>
            <div>
              <Label>Event Code</Label>
              <div>
                <Badge variant="outline">{event.code}</Badge>
              </div>
            </div>
            <div>
              <Label>Customer</Label>
              <div className="text-sm">
                <EventCustomerLink event={event} />
              </div>
            </div>
            <div>
              <Label>Timestamp</Label>
              <div className="text-sm">
                {event.timestamp && timestampDate(event.timestamp).toLocaleString()}
              </div>
            </div>
          </div>

          {Object.keys(event.properties).length > 0 && (
            <div>
              <Label>Properties</Label>
              <Card>
                <CardContent className="p-4">
                  <pre className="text-xs overflow-auto">
                    {stringifyProperties(event.properties, 2)}
                  </pre>
                </CardContent>
              </Card>
            </div>
          )}
        </div>
      )}
    </DialogContent>
  </Dialog>
)

/**
 * Statistical profile of one event source's properties over a range — coverage,
 * cardinality, numeric distribution and top values per key. Opened from an event's
 * Code badge on the Events page to answer "what shape is this stream's data?".
 */
export const EventSourceProfileDialog = ({
  code,
  from,
  to,
  onClose,
}: {
  code: string | null
  from: Date
  to: Date
  onClose: () => void
}) => (
  <Dialog open={!!code} onOpenChange={open => !open && onClose()}>
    <DialogContent className="sm:max-w-2xl max-h-[85vh] overflow-y-auto">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          Properties of <Badge variant="outline">{code}</Badge>
        </DialogTitle>
        <DialogDescription>
          Distribution of the raw-event properties this source carries, over the selected range.
        </DialogDescription>
      </DialogHeader>
      {code && <PropertyProfilePanel eventCode={code} from={from} to={to} />}
    </DialogContent>
  </Dialog>
)
