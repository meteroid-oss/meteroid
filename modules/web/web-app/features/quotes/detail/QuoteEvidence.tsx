import {
  Badge,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { ShieldAlert, ShieldCheck } from 'lucide-react'

import { shortHash } from '@/features/quotes/detail/quoteFormat'
import { useQuery } from '@/lib/connectrpc'
import { listQuoteEvidence } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

const humanize = (kind: string) => {
  const text = kind.replace(/[._]/g, ' ').toLowerCase()
  return text.charAt(0).toUpperCase() + text.slice(1)
}

export const QuoteEvidence = ({ quoteId }: { quoteId: string }) => {
  const query = useQuery(listQuoteEvidence, { quoteId })
  const events = query.data?.events ?? []

  if (query.isLoading) return <Skeleton height={120} />

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-end">
        {events.length > 0 &&
          (query.data?.chainIntact ? (
            <Badge variant="success">
              <ShieldCheck />
              Chain intact
            </Badge>
          ) : (
            <Badge variant="destructive">
              <ShieldAlert />
              Chain broken
            </Badge>
          ))}
      </div>
      {events.length === 0 ? (
        <div className="text-sm text-muted-foreground py-6 text-center">No events yet</div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>#</TableHead>
              <TableHead>Time</TableHead>
              <TableHead>Event</TableHead>
              <TableHead>Actor</TableHead>
              <TableHead>IP</TableHead>
              <TableHead>Hash</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {events.map(event => (
              <TableRow key={event.seq}>
                <TableCell className="text-muted-foreground">{event.seq}</TableCell>
                <TableCell className="whitespace-nowrap">
                  {parseAndFormatDateTime(event.occurredAt, 'sec')}
                </TableCell>
                <TableCell>
                  <div>{humanize(event.kind)}</div>
                  <div className="text-xs text-muted-foreground">Rev {event.revision}</div>
                </TableCell>
                <TableCell className="max-w-[180px] truncate" title={event.userAgent}>
                  {event.actorEmail ?? '—'}
                </TableCell>
                <TableCell className="font-mono text-xs">{event.ipAddress ?? '—'}</TableCell>
                <TableCell className="font-mono text-xs" title={event.hash}>
                  {shortHash(event.hash)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}
