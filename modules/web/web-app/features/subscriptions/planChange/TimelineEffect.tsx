import { Badge, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@md/ui'

import { formatDateOnly } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { TimelineEntry_Effect } from '@/rpc/api/subscriptions/v1/models_pb'

import type { TimelineEffect as Effect } from '@/rpc/api/subscriptions/v1/models_pb'

export interface InvoiceTotal {
  on: string
  total: bigint
}

const money = (amount: bigint, currency: string) =>
  currency ? formatCurrency(amount, currency) : String(amount)

/**
 * The invoices ahead without and with an edit, those that differ: fixed fees before tax and
 * credits, from today (what is already billed left out).
 */
export const ProjectionDiff = ({
  before,
  after,
  currency,
  until,
}: {
  before: InvoiceTotal[]
  after: InvoiceTotal[]
  currency: string
  until: string
}) => {
  const was = new Map(before.map(invoice => [invoice.on, invoice.total]))
  const will = new Map(after.map(invoice => [invoice.on, invoice.total]))
  const dates = [...new Set([...was.keys(), ...will.keys()])].sort()
  const changed = dates.filter(on => (was.get(on) ?? 0n) !== (will.get(on) ?? 0n))
  if (changed.length === 0)
    return (
      <p className="text-sm text-muted-foreground">
        No invoice changes up to {formatDateOnly(until)}.
      </p>
    )
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Invoice</TableHead>
          <TableHead className="text-right">Today</TableHead>
          <TableHead className="text-right">With the change</TableHead>
          <TableHead className="text-right">Difference</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {changed.map(on => {
          const before = was.get(on) ?? 0n
          const after = will.get(on) ?? 0n
          const diff = after - before
          return (
            <TableRow key={on}>
              <TableCell>{formatDateOnly(on)}</TableCell>
              <TableCell className="text-right">{money(before, currency)}</TableCell>
              <TableCell className="text-right">{money(after, currency)}</TableCell>
              <TableCell className="text-right font-medium">
                {diff > 0n ? '+' : ''}
                {money(diff, currency)}
              </TableCell>
            </TableRow>
          )
        })}
      </TableBody>
    </Table>
  )
}

const EFFECT: Record<
  TimelineEntry_Effect,
  { label: string; tone: 'secondary' | 'outline' | 'destructive' }
> = {
  [TimelineEntry_Effect.KEPT]: { label: 'Kept', tone: 'secondary' },
  [TimelineEntry_Effect.INACTIVE]: { label: 'Partly inactive', tone: 'outline' },
  [TimelineEntry_Effect.PRORATED]: { label: 'Settled on its day', tone: 'outline' },
  [TimelineEntry_Effect.DROPPED]: { label: 'Dropped', tone: 'destructive' },
  [TimelineEntry_Effect.SWITCH_REPLACED]: { label: 'Switch replaced', tone: 'outline' },
}

const detail = (effect: TimelineEntry_Effect, details: string[]) => {
  if (details.length === 0) return undefined
  return effect === TimelineEntry_Effect.INACTIVE
    ? `No longer on the subscription from then: ${details.join(', ')}`
    : details.join(', ')
}

/**
 * What a plan change or amendment does to the subscription's timeline: each phase, dated add-on
 * or discount and renewal written again over it (R-162), and the invoices it changes.
 */
export const TimelineEffectView = ({ effect, currency }: { effect: Effect; currency: string }) => (
  <div className="space-y-3">
    <p className="text-sm text-muted-foreground">
      This subscription has phases or dated changes after this one: they are written again over it.
      A price of a fee this change replaces no longer applies; seat floors, minimums and later plan
      switches stay.
    </p>
    <ul className="space-y-1.5">
      {effect.entries.map(entry => {
        const shown = EFFECT[entry.effect]
        const note = detail(entry.effect, entry.details)
        return (
          <li
            key={`${entry.ownerId ?? 'renewal'}-${entry.on}`}
            className="flex flex-wrap items-center gap-2 text-sm"
          >
            <span className="font-medium">{entry.label}</span>
            <span className="text-muted-foreground">{formatDateOnly(entry.on)}</span>
            <Badge variant={shown.tone} size="sm">
              {shown.label}
            </Badge>
            {note && <span className="text-xs text-muted-foreground">{note}</span>}
          </li>
        )
      })}
    </ul>
    <ProjectionDiff
      before={effect.before}
      after={effect.after}
      currency={currency}
      until={effect.until}
    />
  </div>
)
