import { Timestamp } from '@bufbuild/protobuf'
import {
  Badge,
  Button,
  Card,
  CardContent,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { EyeIcon, FileUpIcon, PauseIcon, PlayIcon, RefreshCcwIcon, SearchIcon } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { DateRange } from 'react-day-picker'

import PageHeading from '@/components/PageHeading/PageHeading'
import { StandardTable } from '@/components/table/StandardTable'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { DatePickerWithRange } from '@/features/dashboard/DateRangePicker'
import { EventsImportModal } from '@/features/events/EventsImportModal'
import { useQuery as useConnectQuery } from '@/lib/connectrpc'
import { searchEvents } from '@/rpc/api/events/v1/events-EventsService_connectquery'
import { EventSummary, SearchEventsRequest, SearchEventsRequest_SortOrder } from '@/rpc/api/events/v1/events_pb'

import type { FunctionComponent } from 'react'

const SORT_ORDER_OPTIONS = [
  { value: SearchEventsRequest_SortOrder.TIMESTAMP_DESC, label: 'Newest first' },
  { value: SearchEventsRequest_SortOrder.TIMESTAMP_ASC, label: 'Oldest first' },
  { value: SearchEventsRequest_SortOrder.INGESTED_DESC, label: 'Recently ingested' },
  { value: SearchEventsRequest_SortOrder.INGESTED_ASC, label: 'Oldest ingested' },
]

export const EventsPage: FunctionComponent = () => {
  const [pagination, setPagination] = useState<PaginationState>({ pageIndex: 0, pageSize: 20 })
  const [search, setSearch] = useState('')
  const [customerId, setCustomerId] = useState<string | undefined>()
  const [sortOrder, setSortOrder] = useState(SearchEventsRequest_SortOrder.TIMESTAMP_DESC)
  const [isLive, setIsLive] = useState(false)
  const [newEventIds, setNewEventIds] = useState(new Set<string>())
  const [selectedEvent, setSelectedEvent] = useState<EventSummary | null>(null)
  const [isUploadOpen, setIsUploadOpen] = useState(false)
  const prevEventsRef = useRef<EventSummary[]>([])

  const [dateRange, setDateRange] = useState<DateRange | undefined>(() => {
    const to = new Date()
    const from = new Date()
    from.setDate(from.getDate() - 30)
    return { from, to }
  })

  const searchRequest = useMemo(() => {
    // Default to last 30 days if no range is set
    let from = dateRange?.from || new Date(new Date().getTime() - 30 * 24 * 60 * 60 * 1000)
    let to = dateRange?.to || new Date()

    // inclusive
    from = new Date(from)
    from.setHours(0, 0, 0, 0)
    to = new Date(to)
    to.setHours(23, 59, 59, 999)

    return new SearchEventsRequest({
      from: Timestamp.fromDate(from),
      to: Timestamp.fromDate(to),
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
      search: search || undefined,
      eventCodes: [],
      customerIds: customerId ? [customerId] : [],
      sortOrder,
    })
  }, [pagination, search, customerId, sortOrder, dateRange])

  // Fetch events
  const eventsQuery = useConnectQuery(searchEvents, searchRequest, {
    refetchInterval: isLive ? 5000 : false, // Poll every 5 seconds when live
    staleTime: isLive ? 0 : 30000,
    queryKeyHashFn: () =>
      `events-${pagination.pageIndex}-${pagination.pageSize}-${search}-${customerId}-${sortOrder}-${dateRange?.from?.getTime()}-${dateRange?.to?.getTime()}`,
  })

  // Update "to" date when in live mode
  useEffect(() => {
    if (isLive) {
      const interval = setInterval(() => {
        setDateRange(prev => prev ? { ...prev, to: new Date() } : undefined)
      }, 5000)
      return () => clearInterval(interval)
    }
  }, [isLive])

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
        cell: ({ row }) => <Badge variant="outline">{row.original.code}</Badge>,
      },
      {
        header: 'Customer',
        accessorKey: 'customerId',
        cell: ({ row }) => <div className="font-mono text-xs">{row.original.customerId}</div>,
      },
      {
        header: 'Timestamp',
        accessorKey: 'timestamp',
        cell: ({ row }) => {
          const timestamp = row.original.timestamp
          if (!timestamp) return '-'
          const date = timestamp.toDate()
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
          const s = JSON.stringify(row.original.properties)
          return <div className="text-xs">{s.length > 50 ? s.slice(0, 50) + '...' : s}</div>
        },
      },
      {
        header: 'View',
        cell: ({ row }) => (
          <Button variant="ghost" size="sm" onClick={() => setSelectedEvent(row.original)}>
            <EyeIcon className="h-4 w-4"/>
          </Button>
        ),
        className: 'w-4',
      },
    ],
    []
  )

  return (
    <div className="space-y-6">
      <PageHeading>Events</PageHeading>

      {/* Controls */}
      <div className="flex items-center justify-between">
        <div className="flex items-center space-x-4">
          {/* Search */}
          <div className="relative">
            <SearchIcon className="absolute left-3 top-1/2 transform -translate-y-1/2 h-4 w-4 text-muted-foreground"/>
            <Input
              placeholder="Search events..."
              value={search}
              onChange={e => setSearch(e.target.value)}
              className="pl-10 w-64"
            />
          </div>

          {/* Date Range */}
          <DatePickerWithRange range={dateRange} setRange={setDateRange}/>

          {/* Filters */}
          <div className="flex items-center space-x-2">
            <CustomerSelect
              value={customerId}
              onChange={e => setCustomerId(e)}
              placeholder="Select a customer"
            />
            <Select
              value={sortOrder.toString()}
              onValueChange={value => setSortOrder(parseInt(value))}
            >
              <SelectTrigger className="w-40">
                <SelectValue/>
              </SelectTrigger>
              <SelectContent>
                {SORT_ORDER_OPTIONS.map(option => (
                  <SelectItem key={option.value} value={option.value.toString()}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="flex items-center space-x-2">
          {/* Live/Pause toggle */}
          <Button
            variant={isLive ? 'default' : 'outline'}
            size="sm"
            onClick={() => setIsLive(!isLive)}
          >
            {isLive ? (
              <>
                <PauseIcon className="h-4 w-4 mr-2"/>
                Pause
              </>
            ) : (
              <>
                <PlayIcon className="h-4 w-4 mr-2"/>
                Live
              </>
            )}
          </Button>

          {/* Manual refresh */}
          <Button
            variant="outline"
            size="sm"
            onClick={() => eventsQuery.refetch()}
            disabled={eventsQuery.isFetching}
          >
            <RefreshCcwIcon className={`h-4 w-4 ${eventsQuery.isFetching ? 'animate-spin' : ''}`}/>
          </Button>

          {/* CSV Import */}
          <Button variant="outline" size="sm" onClick={() => setIsUploadOpen(true)}>
            <FileUpIcon className="h-4 w-4 mr-2"/>
            Import CSV
          </Button>
          <EventsImportModal
            openState={[isUploadOpen, setIsUploadOpen]}
            onSuccess={() => eventsQuery.refetch()}
          />
        </div>
      </div>

      {/* Events Table */}
      <StandardTable
        columns={columns}
        data={eventsQuery.data?.events || []}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={eventsQuery.data?.events?.length || 0}
        isLoading={eventsQuery.isLoading}
        emptyMessage="No events found"
        rowClassName={row =>
          newEventIds.has(row.original.id) ? 'animate-pulse bg-green-50 dark:bg-green-950' : ''
        }
      />

      {/* Event Detail Modal */}
      <Dialog open={!!selectedEvent} onOpenChange={() => setSelectedEvent(null)}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Event Details</DialogTitle>
            <DialogDescription>Full event information and properties</DialogDescription>
          </DialogHeader>
          {selectedEvent && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label>Event ID</Label>
                  <div className="font-mono text-sm">{selectedEvent.id}</div>
                </div>
                <div>
                  <Label>Event Code</Label>
                  <div>
                    <Badge variant="outline">{selectedEvent.code}</Badge>
                  </div>
                </div>
                <div>
                  <Label>Customer ID</Label>
                  <div className="font-mono text-sm">{selectedEvent.customerId}</div>
                </div>
                <div>
                  <Label>Timestamp</Label>
                  <div className="text-sm">
                    {selectedEvent.timestamp?.toDate().toLocaleString()}
                  </div>
                </div>
              </div>

              {Object.keys(selectedEvent.properties).length > 0 && (
                <div>
                  <Label>Properties</Label>
                  <Card>
                    <CardContent className="p-4">
                      <pre className="text-xs overflow-auto">
                        {JSON.stringify(selectedEvent.properties, null, 2)}
                      </pre>
                    </CardContent>
                  </Card>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>

      {/* Status indicator */}
      {isLive && (
        <div className="fixed bottom-4 right-4">
          <Badge variant="default" className="animate-pulse">
            Live mode active
          </Badge>
        </div>
      )}
    </div>
  )
}
