import { create } from '@bufbuild/protobuf'
import { timestampDate, timestampFromDate } from '@bufbuild/protobuf/wkt'
import { Badge, Button } from '@md/ui'
import { keepPreviousData } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { format, parseISO } from 'date-fns'
import { EyeIcon, FileUpIcon, PauseIcon, PlayIcon } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { DateRange } from 'react-day-picker'
import { useSearchParams } from 'react-router-dom'

import { Gate } from '@/components/Gate'
import { ListPageLayout, ListToolbarSearch, RefreshButton } from '@/components/layouts'
import { StandardTable } from '@/components/table/StandardTable'
import { BaseFilter } from '@/features/TablePage'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import {
  EventCustomerLink,
  EventDetailsDialog,
  EventSourceProfileDialog,
  stringifyProperties,
} from '@/features/events/EventDetails'
import { EventsImportModal } from '@/features/events/EventsImportModal'
import { useListPagination } from '@/hooks/useTablePreferences'
import { useQuery as useConnectQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { searchEvents } from '@/rpc/api/events/v1/events-EventsService_connectquery'
import {
  EventSummary,
  SearchEventsRequestSchema,
  SearchEventsRequest_SortOrder,
} from '@/rpc/api/events/v1/events_pb'

import type { FunctionComponent } from 'react'

const SORT_ORDER_OPTIONS = [
  { value: SearchEventsRequest_SortOrder.TIMESTAMP_DESC, label: 'Newest first' },
  { value: SearchEventsRequest_SortOrder.TIMESTAMP_ASC, label: 'Oldest first' },
  { value: SearchEventsRequest_SortOrder.INGESTED_DESC, label: 'Recently ingested' },
  { value: SearchEventsRequest_SortOrder.INGESTED_ASC, label: 'Oldest ingested' },
]

const DAY_FORMAT = 'yyyy-MM-dd'

const parseDay = (value: string | null) => {
  if (!value) return undefined
  const date = parseISO(value)
  return Number.isNaN(date.getTime()) ? undefined : date
}

/** The last 30 days, unless the URL (`from` / `to`, YYYY-MM-DD) says otherwise. */
const initialRange = (params: URLSearchParams): DateRange => {
  const from = parseDay(params.get('from'))
  const to = parseDay(params.get('to'))
  if (from || to) return { from, to }
  const end = new Date()
  const start = new Date()
  start.setDate(start.getDate() - 30)
  return { from: start, to: end }
}

export const EventsPage: FunctionComponent = () => {
  // Filters live in the URL (`customer`, `code`, `from`, `to`) so other pages can deep-link here.
  const [params, setParams] = useSearchParams()
  const customerId = params.get('customer') || undefined
  const eventCodes = useMemo(() => (params.get('code') ?? '').split(',').filter(Boolean), [params])
  const setParam = (key: string, value?: string) =>
    setParams(
      prev => {
        const next = new URLSearchParams(prev)
        if (value) next.set(key, value)
        else next.delete(key)
        return next
      },
      { replace: true }
    )
  const setCustomerId = (id?: string) => setParam('customer', id)
  const setEventCodes = (codes: string[]) => setParam('code', codes.join(','))

  const [pagination, setPagination] = useListPagination()
  const [search, setSearch] = useState('')
  const [sortOrder, setSortOrder] = useState(SearchEventsRequest_SortOrder.TIMESTAMP_DESC)
  const [isLive, setIsLive] = useState(false)
  const [newEventIds, setNewEventIds] = useState(new Set<string>())
  const [selectedEvent, setSelectedEvent] = useState<EventSummary | null>(null)
  const [profileCode, setProfileCode] = useState<string | null>(null)
  const [isUploadOpen, setIsUploadOpen] = useState(false)
  const prevEventsRef = useRef<EventSummary[]>([])

  const [dateRange, setDateRangeState] = useState<DateRange | undefined>(() => initialRange(params))
  const setDateRange = (range: DateRange | undefined) => {
    setDateRangeState(range)
    setParams(
      prev => {
        const next = new URLSearchParams(prev)
        for (const [key, day] of [
          ['from', range?.from],
          ['to', range?.to],
        ] as const) {
          if (day) next.set(key, format(day, DAY_FORMAT))
          else next.delete(key)
        }
        return next
      },
      { replace: true }
    )
  }

  const metricsQuery = useConnectQuery(listBillableMetrics, {
    pagination: { perPage: 200, page: 0 },
  })
  const codeOptions = useMemo(() => {
    const codes = new Set((metricsQuery.data?.billableMetrics ?? []).map(m => m.code))
    eventCodes.forEach(c => codes.add(c))
    return [...codes].sort().map(code => ({ label: code, value: code }))
  }, [metricsQuery.data, eventCodes])

  const searchRequest = useMemo(() => {
    // Default to last 30 days if no range is set
    let from = dateRange?.from || new Date(new Date().getTime() - 30 * 24 * 60 * 60 * 1000)
    let to = dateRange?.to || new Date()

    // inclusive
    from = new Date(from)
    from.setHours(0, 0, 0, 0)
    to = new Date(to)
    to.setHours(23, 59, 59, 999)

    return create(SearchEventsRequestSchema, {
      from: timestampFromDate(from),
      to: timestampFromDate(to),
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
      search: search || undefined,
      eventCodes,
      customerIds: customerId ? [customerId] : [],
      sortOrder,
    })
  }, [pagination, search, customerId, eventCodes, sortOrder, dateRange])

  // Normalised range for the per-source property profiler (mirrors the events query
  // default of a trailing 30 days when unset).
  const profileRange = useMemo(() => {
    const from = dateRange?.from ?? new Date(Date.now() - 30 * 24 * 60 * 60 * 1000)
    const to = dateRange?.to ?? new Date()
    return { from, to }
  }, [dateRange])

  // Fetch events. In live mode we poll the same query key every 5s; keeping the
  // previous data across refetches avoids collapsing to skeletons (flicker).
  const eventsQuery = useConnectQuery(searchEvents, searchRequest, {
    refetchInterval: isLive ? 5000 : false, // Poll every 5 seconds when live
    staleTime: isLive ? 0 : 30000,
    placeholderData: keepPreviousData,
    queryKeyHashFn: () =>
      `events-${pagination.pageIndex}-${pagination.pageSize}-${search}-${customerId}-${eventCodes.join(',')}-${sortOrder}-${dateRange?.from?.getTime()}-${dateRange?.to?.getTime()}`,
  })

  // Highlight new events when in live mode
  useEffect(() => {
    if (isLive && eventsQuery.data?.events) {
      const currentEventIds = eventsQuery.data.events.map(e => e.id)
      const prevEventIds = prevEventsRef.current.map(e => e.id)
      const newIds = currentEventIds.filter(id => !prevEventIds.includes(id))

      if (newIds.length > 0) {
        setNewEventIds(new Set(newIds))
        // Clear highlights after 3 seconds
        setTimeout(() => setNewEventIds(new Set()), 3000)
      }

      prevEventsRef.current = eventsQuery.data.events
    }
  }, [eventsQuery.data?.events, isLive])

  // Table columns
  const columns = useMemo<ColumnDef<EventSummary>[]>(
    () => [
      {
        header: 'Event ID',
        accessorKey: 'id',
        cell: ({ row }) => <div className="font-mono text-xs">{row.original.id}</div>,
      },
      {
        header: 'Code',
        accessorKey: 'code',
        cell: ({ row }) => (
          <button
            type="button"
            onClick={() => setProfileCode(row.original.code)}
            title="Profile this event source's properties"
          >
            <Badge variant="outline" className="cursor-pointer hover:bg-muted">
              {row.original.code}
            </Badge>
          </button>
        ),
      },
      {
        header: 'Customer',
        accessorKey: 'customerId',
        cell: ({ row }) => <EventCustomerLink event={row.original} />,
      },
      {
        header: 'Timestamp',
        accessorKey: 'timestamp',
        cell: ({ row }) => {
          const timestamp = row.original.timestamp
          if (!timestamp) return '-'
          const date = timestampDate(timestamp)
          return (
            <div className="text-xs">
              <div>{date.toLocaleDateString()}</div>
              <div className="text-muted-foreground">{date.toLocaleTimeString()}</div>
            </div>
          )
        },
      },
      {
        header: 'Properties',
        cell: ({ row }) => {
          const s = stringifyProperties(row.original.properties)
          return <div className="text-xs">{s.length > 50 ? s.slice(0, 50) + '...' : s}</div>
        },
      },
      {
        header: 'View',
        cell: ({ row }) => (
          <Button variant="ghost" size="sm" onClick={() => setSelectedEvent(row.original)}>
            <EyeIcon className="h-4 w-4" />
          </Button>
        ),
        className: 'w-4',
      },
    ],
    []
  )

  return (
    <ListPageLayout
      title="Events"
      actions={
        // Import runs as a batch job (settings:manage); events:read alone only views.
        <Gate perm="settings:manage">
          <Button variant="secondary" size="sm" onClick={() => setIsUploadOpen(true)}>
            <FileUpIcon className="h-4 w-4 mr-2" />
            Import CSV
          </Button>
        </Gate>
      }
      toolbar={
        <>
          <ListToolbarSearch
            value={search}
            onChange={setSearch}
            placeholder="Search events..."
            className="max-w-[200px]"
          />
          <DatePickerWithRange range={dateRange} setRange={setDateRange} />
          <CustomerSelect
            value={customerId}
            onChange={e => setCustomerId(e || undefined)}
            placeholder="All customers"
            clearable
            variant="filter"
          />
          <BaseFilter
            entries={codeOptions}
            emptyLabel="All event codes"
            selected={eventCodes}
            onSelectionChange={(value, checked) =>
              setEventCodes(
                checked ? [...eventCodes, value] : eventCodes.filter(code => code !== value)
              )
            }
          />
          <BaseFilter
            entries={SORT_ORDER_OPTIONS.map(o => ({ label: o.label, value: o.value.toString() }))}
            emptyLabel="Newest first"
            selected={[sortOrder.toString()]}
            onSelectionChange={(value, checked) =>
              setSortOrder(checked ? parseInt(value) : SearchEventsRequest_SortOrder.TIMESTAMP_DESC)
            }
          />
        </>
      }
      toolbarTrailing={
        <>
          <Button
            variant={isLive ? 'default' : 'outline'}
            size="sm"
            onClick={() => setIsLive(!isLive)}
          >
            {isLive ? (
              <>
                <PauseIcon className="h-4 w-4 mr-2" />
                Pause
              </>
            ) : (
              <>
                <PlayIcon className="h-4 w-4 mr-2" />
                Live
              </>
            )}
          </Button>
          <RefreshButton isLoading={eventsQuery.isFetching} onClick={() => eventsQuery.refetch()} />
        </>
      }
    >
      <StandardTable
        tableKey="events"
        columns={columns}
        data={eventsQuery.data?.events || []}
        getRowId={event => event.id}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={eventsQuery.data?.events?.length || 0}
        isLoading={eventsQuery.isLoading}
        emptyMessage="No events found"
        rowClassName={row =>
          newEventIds.has(row.original.id) ? 'animate-pulse bg-green-50 dark:bg-green-950' : ''
        }
      />

      <EventsImportModal
        openState={[isUploadOpen, setIsUploadOpen]}
        onSuccess={() => eventsQuery.refetch()}
      />

      {/* Event Detail Modal */}
      <EventDetailsDialog event={selectedEvent} onClose={() => setSelectedEvent(null)} />

      {/* Per-source property profiler, opened from a Code badge */}
      <EventSourceProfileDialog
        code={profileCode}
        from={profileRange.from}
        to={profileRange.to}
        onClose={() => setProfileCode(null)}
      />

      {/* Status indicator */}
      {isLive && (
        <div className="fixed bottom-4 right-4">
          <Badge variant="default" className="animate-pulse">
            Live mode active
          </Badge>
        </div>
      )}
    </ListPageLayout>
  )
}
