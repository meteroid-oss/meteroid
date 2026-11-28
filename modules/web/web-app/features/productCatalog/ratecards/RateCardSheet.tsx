import {
  createQueryOptions,
  skipToken,
  useTransport,
} from '@connectrpc/connect-query'
import {
  Button,
  DatePicker,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Input,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
  cn,
} from '@md/ui'
import { useQueries, useQueryClient } from '@tanstack/react-query'
import { format } from 'date-fns'
import { MoreHorizontalIcon, PlusIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { useCreditUnit } from '@/features/pricing/CreditUnitPriceFields'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { downloadCsv } from '@/lib/utils/csv'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { discoverPropertyValues } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'
import { RateCardUnmatched, type RateCard } from '@/rpc/api/ratecards/v1/models_pb'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { RateCardHistory } from './RateCardHistory'
import { shortDay } from './history'
import { parseRateLines, type RateLine } from './rateLines'
import {
  CREDITS,
  acceptRateInput,
  buildRows,
  changeOf,
  columnsOf,
  currencySymbol,
  dimensionName,
  draftRows,
  emptyDraft,
  exportLines,
  importLines,
  isMoney,
  listNames,
  newRowId,
  rateCardHeading,
  sameRate,
  unitOf,
  unitSizeLabel,
  type Draft,
  type Row,
} from './rateTable'
import { coreRateCards, type RateCardService } from './service'

const isoDay = (date: Date) => format(date, 'yyyy-MM-dd')

export const unmatchedLabel = (card: RateCard) =>
  card.onUnmatched === RateCardUnmatched.IGNORE ? 'Left unbilled' : 'Holds the invoice'

/** The values seen of each dimension on the metric's events, most frequent first. */
function useSeenValues(eventCode: string | undefined, dimensions: string[]) {
  const transport = useTransport()
  const results = useQueries({
    queries: (eventCode ? dimensions : []).map(dimension =>
      createQueryOptions(
        discoverPropertyValues,
        { eventCode: eventCode ?? '', propertyKey: dimension, limit: 50 },
        { transport }
      )
    ),
  })
  return Object.fromEntries(
    dimensions.map((dimension, i) => [
      dimension,
      results[i]?.data?.values.map(v => v.value).filter(Boolean) ?? [],
    ])
  ) as Record<string, string[]>
}

/**
 * A card's rates as one table, a row per key and a column per currency (or its credit unit).
 * Edits are staged and saved as one batch, from today or a day picked. Its history shows each
 * row's rates over time and who changed them.
 */
export const RateCardSheet = ({
  id,
  onClose,
  service = coreRateCards,
}: {
  id: string
  onClose: () => void
  service?: RateCardService
}) => {
  const [today] = useState(() => isoDay(new Date()))
  const cardQuery = service.useCard(id)
  const card = cardQuery.data?.card
  const metric = useQuery(getBillableMetric, card ? { id: card.metricId } : skipToken).data
    ?.billableMetric
  const tenantCurrencies = useQuery(listTenantCurrencies).data?.currencies
  const creditUnit = useCreditUnit(card?.creditUnitId)
  const canEdit = usePermissions().has('catalog:manage')
  const seen = useSeenValues(metric?.code, card?.dimensions ?? [])

  const [draft, setDraft] = useState<Draft>(emptyDraft)
  const [from, setFrom] = useState<Date | undefined>()
  const [importing, setImporting] = useState(false)
  const [view, setView] = useState<'rates' | 'history'>('rates')
  // The row whose history was asked for, to open the history on it.
  const [historyOf, setHistoryOf] = useState<string[]>()
  const queryClient = useQueryClient()
  // The cell whose last keystroke was refused, to say why it did nothing.
  const [refused, setRefused] = useState<string>()

  const rows = useMemo(
    () =>
      card ? buildRows(card, cardQuery.data?.rates ?? [], cardQuery.data?.entries ?? [], today) : [],
    [card, cardQuery.data, today]
  )
  const shown = draftRows(rows, draft)
  const columns = card ? columnsOf(card, tenantCurrencies ?? [], rows, draft) : []
  const change = useMemo(() => changeOf(rows, draft), [rows, draft])
  const count = change.rates.length + change.remove.length
  const invalid = Object.keys(change.errors).length > 0
  const backdated = from !== undefined && isoDay(from) < today
  const staged = count > 0 || invalid || draft.added.length > 0 || draft.removed.length > 0
  const empty = !shown.some(row => row.added || Object.keys(row.cells).length > 0)

  const save = service.useSetRates({
    onSuccess: result => {
      const done = [
        result.inserted && `${result.inserted} set`,
        result.removed && `${result.removed} removed`,
        result.replaced && `${result.replaced} scheduled replaced`,
      ].filter(Boolean)
      toast.success(done.length ? `Rates saved: ${done.join(', ')}` : 'Nothing changed')
      setDraft(emptyDraft)
      setFrom(undefined)
      cardQuery.refetch()
      queryClient.invalidateQueries({ queryKey: service.changesKey })
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The rates could not be saved')),
  })

  const close = () => {
    if (staged && !window.confirm('Discard the rates not saved?')) return
    onClose()
  }

  const edit = (rowId: string, column: string, text: string) => {
    const accepted = acceptRateInput(text)
    setRefused(accepted === undefined ? `${rowId}/${column}` : undefined)
    if (accepted === undefined) return
    setDraft(d => ({
      ...d,
      edits: { ...d.edits, [rowId]: { ...d.edits[rowId], [column]: accepted } },
    }))
  }
  const setKey = (rowId: string, index: number, pattern: string) =>
    setDraft(d => ({
      ...d,
      added: d.added.map(a =>
        a.id === rowId ? { ...a, key: a.key.map((p, i) => (i === index ? pattern : p)) } : a
      ),
    }))
  const addRow = () => {
    if (!card) return
    setDraft(d => ({
      ...d,
      added: [...d.added, { id: newRowId(), key: card.dimensions.map(() => '') }],
    }))
  }
  const remove = (row: Row) =>
    setDraft(d => {
      if (!row.added) return { ...d, removed: [...d.removed, row.id] }
      const edits = { ...d.edits }
      delete edits[row.id]
      return { ...d, edits, added: d.added.filter(a => a.id !== row.id) }
    })
  const restore = (row: Row) =>
    setDraft(d => ({ ...d, removed: d.removed.filter(r => r !== row.id) }))
  const showHistory = (key?: string[]) => {
    setHistoryOf(key)
    setView('history')
  }

  const exportCsv = () => {
    if (!card) return
    const header = [...card.dimensions, ...(isMoney(card) ? ['currency'] : []), 'rate']
    downloadCsv(`${card.code}-rates.csv`, header, exportLines(card, rows))
  }

  const creditName = creditUnit?.pluralName || creditUnit?.name
  const per = card ? unitSizeLabel(card.unitSize, unitOf(metric?.unitLabel)) : ''
  const columnLabel = (column: string) => (column === CREDITS ? creditName || 'Credits' : column)
  const amount = (rate: string, column: string) =>
    column === CREDITS ? rate : formatCurrencyNoRounding(rate, column)
  const sample = (dimension: string) => seen[dimension]?.[0]

  const emptyText = () => {
    if (!card) return ''
    const each = `No rates yet. Add a row for each ${listNames(card.dimensions.map(dimensionName))}`
    if (!card.dimensions.some(sample)) return `${each} with its rate ${per}.`
    const key = card.dimensions.map(d => sample(d) ?? 'any').join(' · ')
    const rate =
      columns[0] && columns[0] !== CREDITS
        ? formatCurrencyNoRounding('2.5', columns[0])
        : `2.50${creditName ? ` ${creditName}` : ''}`
    return `${each}, e.g. ${key} → ${rate} ${per}.`
  }

  const unmatchedHint =
    card?.onUnmatched === RateCardUnmatched.IGNORE
      ? 'Leave it empty to leave that usage unbilled.'
      : 'Leave it empty to hold the invoice until you price it.'

  return (
    <Sheet open onOpenChange={open => !open && close()}>
      <SheetContent size="large" className="flex flex-col gap-0 p-0">
        <SheetHeader className="space-y-1 border-b border-border px-6 py-4 pr-12">
          <div className="flex items-center justify-between gap-4">
            <SheetTitle>{card?.name ?? 'Rate card'}</SheetTitle>
            {card && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="icon" className="h-7 w-7" aria-label="More">
                    <MoreHorizontalIcon className="h-4 w-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  {canEdit && (
                    <DropdownMenuItem onClick={() => setImporting(true)}>
                      Import CSV…
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem onClick={exportCsv}>Export CSV</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
          {card && (
            <SheetDescription>
              <span className="font-mono">{card.code}</span>
            </SheetDescription>
          )}
        </SheetHeader>

        <div className="flex-1 overflow-auto px-6 py-4">
          {cardQuery.isLoading || !card ? (
            <div className="p-4 text-center">
              <Spinner />
            </div>
          ) : (
            <Tabs
              value={view}
              onValueChange={value => {
                setView(value as 'rates' | 'history')
                setHistoryOf(undefined)
              }}
            >
              <TabsList variant="ghost" className="mb-3">
                <TabsTrigger value="rates">Rates</TabsTrigger>
                <TabsTrigger value="history">History</TabsTrigger>
              </TabsList>
              <TabsContent value="rates" className="mt-0">
                {card.dimensions.map(dimension => (
                  <datalist key={dimension} id={`rate-card-values-${dimension}`}>
                    {seen[dimension]?.map(value => <option key={value} value={value} />)}
                  </datalist>
                ))}
                <p className="mb-3 text-sm">
                  {rateCardHeading(card, metric?.unitLabel, creditName)}
                </p>
                {empty ? (
                  <div className="rounded-md border border-dashed border-border px-4 py-8 text-center">
                    <p className="text-sm text-muted-foreground">{emptyText()}</p>
                    {canEdit && (
                      <Button variant="outline" size="sm" hasIcon className="mt-3" onClick={addRow}>
                        <PlusIcon className="h-4 w-4" />
                        Add a row
                      </Button>
                    )}
                  </div>
                ) : (
                  <>
                    <Table containerClassName="rounded-md border border-border">
                      <TableHeader>
                        <TableRow>
                          {card.dimensions.map(dimension => (
                            <TableHead key={dimension}>{dimensionName(dimension)}</TableHead>
                          ))}
                          {columns.map(column => (
                            <TableHead key={column} className="h-auto w-36 py-2 text-right">
                              <div>{columnLabel(column)}</div>
                              <div className="text-[11px] font-normal">{per}</div>
                            </TableHead>
                          ))}
                          <TableHead className="w-10" />
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {shown.map(row => {
                          const removed = draft.removed.includes(row.id)
                          // A stored row with rates has a history; one added in the draft none.
                          const rated = !row.added && Object.keys(row.cells).length > 0
                          const label = Object.values(row.cells).find(c => c.label)?.label
                          return (
                            <TableRow
                              key={row.id}
                              className={cn(
                                removed && 'opacity-50',
                                row.anyOther && 'border-t-2 bg-muted/30'
                              )}
                            >
                              {row.anyOther ? (
                                <TableCell colSpan={card.dimensions.length} className="align-top">
                                  <span className={cn('font-medium', removed && 'line-through')}>
                                    Anything else
                                  </span>
                                  <p className="text-[11px] text-muted-foreground">
                                    Usage no row above matches. {unmatchedHint}
                                  </p>
                                </TableCell>
                              ) : row.added ? (
                                card.dimensions.map((dimension, i) => (
                                  <TableCell key={dimension} className="align-top">
                                    <Input
                                      className="h-8 font-mono"
                                      placeholder={`any${sample(dimension) ? ` (e.g. ${sample(dimension)})` : ""}`}
                                      list={`rate-card-values-${dimension}`}
                                      value={row.key[i]}
                                      onChange={event => setKey(row.id, i, event.target.value)}
                                      autoFocus={i === 0}
                                    />
                                    {i === 0 && change.errors[row.id] && (
                                      <p className="mt-1 text-[11px] text-destructive">
                                        {change.errors[row.id]}
                                      </p>
                                    )}
                                  </TableCell>
                                ))
                              ) : (
                                row.key.map((pattern, i) => (
                                  <TableCell key={i} className="align-top">
                                    {pattern === '*' ? (
                                      <span
                                        className={cn(
                                          'text-muted-foreground',
                                          removed && 'line-through'
                                        )}
                                      >
                                        any
                                      </span>
                                    ) : (
                                      <span className={cn('font-mono', removed && 'line-through')}>
                                        {pattern}
                                      </span>
                                    )}
                                    {i === 0 && label && (
                                      <p className="text-[11px] text-muted-foreground">{label}</p>
                                    )}
                                  </TableCell>
                                ))
                              )}
                              {columns.map(column => {
                                const cellId = `${row.id}/${column}`
                                const cell = row.cells[column]
                                const typed = draft.edits[row.id]?.[column]
                                const value = typed ?? cell?.rate ?? ''
                                const changed =
                                  typed !== undefined &&
                                  !sameRate(typed.trim() || undefined, cell?.rate)
                                const error =
                                  refused === cellId ? 'Not a number' : change.errors[cellId]
                                const symbol = column === CREDITS ? '' : currencySymbol(column)
                                return (
                                  <TableCell key={column} className="align-top">
                                    <div className="relative ml-auto w-32">
                                      {symbol && (
                                        <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                                          {symbol}
                                        </span>
                                      )}
                                      <Input
                                        type="text"
                                        inputMode="decimal"
                                        aria-label={`${columnLabel(column)} ${per}`}
                                        className={cn(
                                          'h-8 text-right font-mono tabular-nums',
                                          symbol && (symbol.length > 1 ? 'pl-10' : 'pl-6'),
                                          changed && 'border-primary',
                                          error && 'border-destructive'
                                        )}
                                        placeholder="—"
                                        value={value}
                                        disabled={!canEdit || removed}
                                        onChange={event => edit(row.id, column, event.target.value)}
                                        onBlur={() => refused === cellId && setRefused(undefined)}
                                      />
                                    </div>
                                    {error && (
                                      <p className="mt-1 text-right text-[11px] text-destructive">
                                        {error}
                                      </p>
                                    )}
                                    {cell?.scheduled.map(s => (
                                      <p
                                        key={s.from}
                                        className="mt-1 text-right text-[11px] text-muted-foreground"
                                      >
                                        → {s.rate === undefined ? 'removed' : amount(s.rate, column)}{' '}
                                        from {shortDay(s.from)}
                                      </p>
                                    ))}
                                  </TableCell>
                                )
                              })}
                              <TableCell className="align-top">
                                {(rated || (canEdit && (removed || row.added))) && (
                                  <DropdownMenu>
                                      <DropdownMenuTrigger asChild>
                                        <Button
                                          variant="ghost"
                                          size="icon"
                                          className="h-8 w-8"
                                          aria-label="Row actions"
                                        >
                                          <MoreHorizontalIcon className="h-4 w-4" />
                                        </Button>
                                      </DropdownMenuTrigger>
                                      <DropdownMenuContent align="end">
                                        {rated && (
                                          <DropdownMenuItem onClick={() => showHistory(row.key)}>
                                            History
                                          </DropdownMenuItem>
                                        )}
                                        {canEdit &&
                                          (removed ? (
                                            <DropdownMenuItem onClick={() => restore(row)}>
                                              Restore
                                            </DropdownMenuItem>
                                          ) : (
                                            <DropdownMenuItem onClick={() => remove(row)}>
                                              Remove
                                            </DropdownMenuItem>
                                          ))}
                                      </DropdownMenuContent>
                                    </DropdownMenu>
                                  )}
                              </TableCell>
                            </TableRow>
                          )
                        })}
                      </TableBody>
                    </Table>
                    {canEdit && (
                      <Button variant="ghost" size="sm" hasIcon className="mt-2" onClick={addRow}>
                        <PlusIcon className="h-4 w-4" />
                        Add a row
                      </Button>
                    )}
                    <p className="mt-4 text-xs text-muted-foreground">
                      A cell left empty matches any value; a value ending in * matches every value
                      it starts with. When rows overlap, the most specific one applies.
                    </p>
                  </>
                )}
              </TabsContent>
              <TabsContent value="history" className="mt-0">
                <RateCardHistory
                  key={historyOf?.join('\u0001') ?? ''}
                  cardId={card.id}
                  service={service}
                  entries={cardQuery.data?.entries ?? []}
                  today={today}
                  amount={amount}
                  focus={historyOf}
                />
              </TabsContent>
            </Tabs>
          )}
        </div>

        {canEdit && card && (view === 'rates' || staged) && (
          <div className="flex flex-wrap items-center gap-3 border-t border-border px-6 py-3">
            {staged ? (
              <>
                <span className="text-sm">
                  {count
                    ? `${count} ${count === 1 ? 'change applies' : 'changes apply'}`
                    : 'Changes apply'}{' '}
                  to usage from
                </span>
                <DatePicker
                  mode="single"
                  placeholder="Today"
                  className="h-8 w-40"
                  date={from}
                  onSelect={setFrom}
                  disabled={change.firstRatesOnly ? undefined : { before: new Date() }}
                />
                {backdated && !change.firstRatesOnly && (
                  <span className="text-xs text-destructive">
                    A change to an existing rate starts today or later.
                  </span>
                )}
                <div className="ml-auto flex gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setDraft(emptyDraft)
                      setFrom(undefined)
                      setRefused(undefined)
                    }}
                  >
                    Discard
                  </Button>
                  <Button
                    size="sm"
                    disabled={
                      !count || invalid || (backdated && !change.firstRatesOnly) || save.isPending
                    }
                    onClick={() =>
                      save.mutate({
                        rateCardId: card.id,
                        effectiveFrom: from ? isoDay(from) : undefined,
                        rates: change.rates,
                        remove: change.remove,
                      })
                    }
                  >
                    Save
                  </Button>
                </div>
              </>
            ) : (
              <span className="text-xs text-muted-foreground">
                Type a rate to set it, clear it to remove it. Nothing changes until you save.
              </span>
            )}
          </div>
        )}

        {importing && card && (
          <ImportDialog
            card={card}
            onClose={() => setImporting(false)}
            onImport={lines => {
              setDraft(d => importLines(rows, d, lines))
              setImporting(false)
              toast.success(`${lines.length} rates staged: save to apply them`)
            }}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

const ImportDialog = ({
  card,
  onClose,
  onImport,
}: {
  card: RateCard
  onClose: () => void
  onImport: (lines: RateLine[]) => void
}) => {
  const [text, setText] = useState('')
  const money = isMoney(card)
  const shape = [...card.dimensions, ...(money ? ['currency'] : []), 'rate'].join(', ')

  const submit = () => {
    const parsed = parseRateLines(text, card.dimensions.length, money)
    if ('error' in parsed) toast.error(parsed.error)
    else onImport(parsed.lines)
  }

  return (
    <Dialog open onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>Import rates</DialogTitle>
          <DialogDescription>
            One rate per line: {shape}. The rates fill the table; nothing is saved until you save
            it.
          </DialogDescription>
        </DialogHeader>
        <Textarea
          rows={8}
          className="font-mono text-xs"
          value={text}
          onChange={event => setText(event.target.value)}
          placeholder={[...card.dimensions.map(() => '*'), ...(money ? ['USD'] : []), '2.50'].join(
            ','
          )}
        />
        <Input
          type="file"
          accept=".csv,.tsv,text/csv,text/plain"
          onChange={async event => {
            const file = event.target.files?.[0]
            if (file) setText(await file.text())
          }}
        />
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={!text.trim()}>
            Fill the table
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
