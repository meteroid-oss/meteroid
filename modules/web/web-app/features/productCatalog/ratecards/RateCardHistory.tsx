import { Badge, Button, Input, Skeleton, cn } from '@md/ui'
import { formatDistanceToNow } from 'date-fns'
import { ArrowRightIcon } from 'lucide-react'
import { useMemo, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { ActorType } from '@/rpc/api/activity/v1/activity_pb'
import { listRateCardChanges } from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'
import { RateCardChangeKind, type RateCardChange } from '@/rpc/api/ratecards/v1/ratecards_pb'

import {
  filterHistory,
  keyLabel,
  rateHistory,
  shortDay,
  withdrawal,
  type KeyHistory,
} from './history'
import { CREDITS } from './rateTable'

import type { RateCardEntry } from '@/rpc/api/ratecards/v1/models_pb'

/** A rate as the sheet shows it in a column: a currency, or the card's credit unit. */
type Amount = (rate: string, column: string) => string

const CHANGES_PAGE = 20
const LINES_SHOWN = 5

/**
 * A card's rates over time, a line per key and currency, and its change log: who set which rates
 * when, the scheduled ones replaced or withdrawn before their day included.
 */
export const RateCardHistory = ({
  cardId,
  entries,
  today,
  amount,
  focus,
}: {
  cardId: string
  entries: RateCardEntry[]
  today: string
  amount: Amount
  /** A row whose history to show first: the filter starts on its key. */
  focus?: string[]
}) => {
  const [filter, setFilter] = useState(() => (focus ? keyLabel(focus) : ''))
  const histories = useMemo(() => rateHistory(entries, today), [entries, today])
  const shown = filterHistory(histories, filter)

  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <div className="flex items-end justify-between gap-4">
          <div>
            <h3 className="text-sm font-medium">Rates over time</h3>
            <p className="text-xs text-muted-foreground">
              Every rate each row has had. A change applies from its day: earlier days keep the
              rates they were billed at.
            </p>
          </div>
          {(histories.length > 6 || filter) && (
            <Input
              className="h-8 w-48 shrink-0"
              placeholder="Filter rows"
              value={filter}
              onChange={event => setFilter(event.target.value)}
            />
          )}
        </div>
        {shown.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {histories.length ? 'No row matches.' : 'No rates yet.'}
          </p>
        ) : (
          <div className="divide-y divide-border rounded-md border border-border">
            {shown.map(history => (
              <KeyTimeline key={history.id} history={history} amount={amount} />
            ))}
          </div>
        )}
      </section>
      <ChangeLog cardId={cardId} amount={amount} />
    </div>
  )
}

const KeyTimeline = ({ history, amount }: { history: KeyHistory; amount: Amount }) => {
  const column = history.currency ?? CREDITS
  return (
    <div className="flex gap-4 px-3 py-2">
      <div className="w-44 shrink-0 min-w-0">
        <div className={cn('truncate text-sm', !history.anyOther && 'font-mono')}>
          {keyLabel(history.key)}
        </div>
        {history.currency && (
          <div className="text-[11px] text-muted-foreground">{history.currency}</div>
        )}
      </div>
      <ol className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
        {history.steps.map((step, i) => (
          <li key={step.id} className="flex items-center gap-1.5">
            {i > 0 && <ArrowRightIcon className="h-3 w-3 text-muted-foreground" />}
            <span
              className={cn(
                'font-mono tabular-nums',
                step.standing === 'past' && 'text-muted-foreground',
                step.standing === 'current' && 'font-medium'
              )}
            >
              {step.rate === undefined ? 'removed' : amount(step.rate, column)}
            </span>
            <span className="text-[11px] text-muted-foreground">{shortDay(step.from)}</span>
            {step.standing === 'current' && step.rate !== undefined && (
              <Badge variant="secondary" size="sm">
                In force
              </Badge>
            )}
            {step.standing === 'scheduled' && (
              <Badge variant="outline" size="sm">
                Scheduled
              </Badge>
            )}
          </li>
        ))}
      </ol>
    </div>
  )
}

type Page = { changes: RateCardChange[]; cursor?: string }

const ChangeLog = ({ cardId, amount }: { cardId: string; amount: Amount }) => {
  // Each "Load more" keeps the page shown and reads the next one, as the activity timeline does.
  const [pages, setPages] = useState<Page[]>([])
  const query = useQuery(listRateCardChanges, {
    rateCardId: cardId,
    cursor: pages.at(-1)?.cursor,
    limit: CHANGES_PAGE,
  })
  const live = query.data?.changes ?? []
  const changes = [...pages.flatMap(page => page.changes), ...live]
  const next = query.data?.nextCursor

  return (
    <section className="space-y-2">
      <div>
        <h3 className="text-sm font-medium">Changes</h3>
        <p className="text-xs text-muted-foreground">
          Who changed the rates, and when. A scheduled rate replaced before its day stays here.
        </p>
      </div>
      {query.isLoading && pages.length === 0 ? (
        <div className="space-y-2 py-2">
          <Skeleton height={14} width={240} />
          <Skeleton height={14} width={200} />
        </div>
      ) : changes.length === 0 ? (
        <p className="text-sm text-muted-foreground">No changes recorded yet.</p>
      ) : (
        <ul className="divide-y divide-border rounded-md border border-border">
          {changes.map(change => (
            <ChangeRow key={change.id} change={change} amount={amount} />
          ))}
        </ul>
      )}
      {next && (
        <Button
          variant="outline"
          size="sm"
          disabled={query.isFetching}
          onClick={() => setPages(p => [...p, { changes: live, cursor: next }])}
        >
          {query.isFetching ? 'Loading…' : 'Load more'}
        </Button>
      )}
    </section>
  )
}

function actorName(change: RateCardChange): string {
  if (change.actorName) return change.actorName
  switch (change.actorType) {
    case ActorType.SYSTEM:
      return 'System'
    case ActorType.API_TOKEN:
      return 'API token'
    case ActorType.OAUTH_APP:
      return 'OAuth app'
    case ActorType.WORKFLOW:
      return 'Workflow'
    default:
      return 'User'
  }
}

const ChangeRow = ({ change, amount }: { change: RateCardChange; amount: Amount }) => {
  const [open, setOpen] = useState(false)
  const occurred = new Date(change.occurredAt)
  const title =
    change.kind === RateCardChangeKind.CREATED
      ? 'Card created'
      : change.effectiveFrom
        ? `Rates from ${shortDay(change.effectiveFrom)}`
        : 'Rates changed'

  const lines = [
    ...change.rates.map((rate, i) => {
      const column = rate.currency ?? CREDITS
      return (
        <li key={`rate-${i}`} className="flex gap-3">
          <KeyCell keyPatterns={rate.key} currency={rate.currency} />
          <span className="shrink-0 font-mono tabular-nums">
            {rate.fromRate === undefined ? (
              <span className="font-sans text-muted-foreground">new · </span>
            ) : (
              <>
                <span className="text-muted-foreground">{amount(rate.fromRate, column)}</span>
                {' → '}
              </>
            )}
            {rate.toRate === undefined ? (
              <span className="font-sans">removed</span>
            ) : (
              amount(rate.toRate, column)
            )}
          </span>
        </li>
      )
    }),
    ...change.withdrawn.map(entry => (
      <li key={`withdrawn-${entry.id}`} className="flex gap-3 text-muted-foreground">
        <KeyCell keyPatterns={entry.key} currency={entry.currency} />
        <span className="shrink-0">
          scheduled{' '}
          <span className="font-mono tabular-nums line-through">
            {entry.rate === undefined
              ? 'removal'
              : amount(entry.rate, entry.currency ?? CREDITS)}
          </span>{' '}
          from {shortDay(entry.effectiveFrom)}{' '}
          {withdrawal(change.rates, change.effectiveFrom, entry)}
        </span>
      </li>
    )),
  ]

  return (
    <li className="px-3 py-2">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-[13px] font-medium">{title}</span>
        <span className="whitespace-nowrap text-[11px] text-muted-foreground">
          <span className="font-medium text-foreground/80">{actorName(change)}</span> ·{' '}
          <time dateTime={change.occurredAt} title={occurred.toLocaleString()}>
            {formatDistanceToNow(occurred, { addSuffix: true })}
          </time>
        </span>
      </div>
      {lines.length > 0 && (
        <ul className="mt-1 space-y-0.5 text-xs">{open ? lines : lines.slice(0, LINES_SHOWN)}</ul>
      )}
      {lines.length > LINES_SHOWN && (
        <button
          type="button"
          className="mt-1 text-[11px] text-muted-foreground hover:text-foreground"
          onClick={() => setOpen(o => !o)}
        >
          {open ? 'Show less' : `${lines.length - LINES_SHOWN} more`}
        </button>
      )}
    </li>
  )
}

const KeyCell = ({ keyPatterns, currency }: { keyPatterns: string[]; currency?: string }) => (
  <span className="min-w-0 flex-1 truncate">
    <span className={cn(!keyPatterns.every(p => p === '*') && 'font-mono')}>
      {keyLabel(keyPatterns)}
    </span>
    {currency && <span className="ml-1.5 text-muted-foreground">{currency}</span>}
  </span>
)
