import { Timestamp } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  CardContent,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  Input,
  Label,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import {
  AlertCircleIcon,
  CheckCircleIcon,
  EyeIcon,
  FileSpreadsheetIcon,
  FileUpIcon,
  PauseIcon,
  PlayIcon,
  RefreshCcwIcon,
  SearchIcon,
  XCircleIcon,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { toast } from 'sonner'

import PageHeading from '@/components/PageHeading/PageHeading'
import { StandardTable } from '@/components/table/StandardTable'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { useQuery as useConnectQuery } from '@/lib/connectrpc'
import {
  ingestEventsFromCsv,
  searchEvents,
} from '@/rpc/api/events/v1/events-EventsIngestionService_connectquery'
import {
  EventSummary,
  FileData,
  IngestEventsFromCsvRequest,
  SearchEventsRequest,
  SearchEventsRequest_SortOrder,
} from '@/rpc/api/events/v1/events_pb'

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
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState({
    delimiter: ',',
    allowBackfilling: true,
    failOnError: false,
  })
  const [importResult, setImportResult] = useState<{
    totalRows: number
    successful: number
    failures: Array<{ rowNumber: number; eventId: string; reason: string }>
  } | null>(null)
  const prevEventsRef = useRef<EventSummary[]>([])

  // Build search request
  const searchRequest = useMemo(() => {
    const now = new Date()
    const from = new Date(now.getTime() - 30 * 24 * 60 * 60 * 1000) // 30 days ago

    return new SearchEventsRequest({
      from: Timestamp.fromDate(from),
      to: Timestamp.fromDate(now),
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
      search: search || undefined,
      eventCodes: [],
      customerIds: customerId ? [customerId] : [],
      sortOrder,
    })
  }, [pagination, search, customerId, sortOrder])

  // Fetch events
  const eventsQuery = useConnectQuery(searchEvents, searchRequest, {
    refetchInterval: isLive ? 5000 : false, // Poll every 5 seconds when live
    staleTime: isLive ? 0 : 30000,
    queryKeyHashFn: () =>
      `events-${pagination.pageIndex}-${pagination.pageSize}-${search}-${customerId}-${sortOrder}`,
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

  // CSV upload mutation
  const uploadMutation = useMutation(ingestEventsFromCsv, {
    onSuccess: async response => {
      const { totalRows, successfulEvents, failures } = response

      setImportResult({
        totalRows,
        successful: successfulEvents,
        failures:
          failures?.map(f => ({
            rowNumber: f.rowNumber,
            eventId: f.eventId,
            reason: f.reason,
          })) || [],
      })

      // Only close modal and refetch if completely successful
      if (!failures || failures.length === 0) {
        setIsUploadOpen(false)
        setUploadFile(null)
        setImportResult(null)
        await eventsQuery.refetch()
        toast.success(`Successfully imported ${successfulEvents} events`)
      }
    },
    onError: error => {
      setImportResult({
        totalRows: 0,
        successful: 0,
        failures: [
          {
            rowNumber: 0,
            eventId: '',
            reason: error.message,
          },
        ],
      })
    },
  })

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

  const handleFileUpload = async () => {
    if (!uploadFile) return

    setImportResult(null) // Clear previous results
    const buffer = await uploadFile.arrayBuffer()
    const request = new IngestEventsFromCsvRequest({
      file: new FileData({ data: new Uint8Array(buffer) }),
      delimiter: csvOptions.delimiter,
      allowBackfilling: csvOptions.allowBackfilling,
      failOnError: csvOptions.failOnError,
    })

    uploadMutation.mutate(request)
  }

  const handleCloseModal = () => {
    setIsUploadOpen(false)
    setUploadFile(null)
    setImportResult(null)
  }

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
          <Dialog open={isUploadOpen} onOpenChange={setIsUploadOpen}>
            <DialogTrigger asChild>
              <Button variant="outline" size="sm">
                <FileUpIcon className="h-4 w-4 mr-2"/>
                Import CSV
              </Button>
            </DialogTrigger>
            <DialogContent className="sm:max-w-2xl max-h-[90vh]">
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <FileSpreadsheetIcon className="h-5 w-5"/>
                  Import Events from CSV
                </DialogTitle>
                <DialogDescription>
                  Import event data from a CSV file. All imports must include headers.
                </DialogDescription>
              </DialogHeader>

              <div className="space-y-6">
                {!importResult ? (
                  <>
                    {/* File Upload */}
                    <div className="space-y-3">
                      <Label className="text-sm font-medium">CSV File</Label>
                      <div
                        className="border-2 border-dashed border-muted-foreground/25 rounded-lg p-4 hover:border-muted-foreground/50 transition-colors">
                        <Input
                          id="file"
                          type="file"
                          accept=".csv,.txt"
                          onChange={e => setUploadFile(e.target.files?.[0] || null)}
                          className="cursor-pointer"
                        />
                        {uploadFile && (
                          <div className="mt-2 text-sm text-muted-foreground">
                            Selected: {uploadFile.name} ({(uploadFile.size / 1024).toFixed(1)} KB)
                          </div>
                        )}
                      </div>
                    </div>

                    {/* CSV Format Requirements */}
                    <Card>
                      <CardContent className="p-4 space-y-3">
                        <div className="grid grid-cols-1 gap-3 text-sm">
                          <div className="grid grid-cols-2">
                            <strong>Required columns:</strong>
                            <div className="mt-1 flex gap-2 flex-wrap">
                              <Badge variant="outline" className="text-xs">
                                event_code
                              </Badge>
                              <Badge variant="outline" className="text-xs">
                                customer_id
                              </Badge>
                            </div>
                          </div>
                          <div className="grid grid-cols-2">
                            <strong>Optional columns:</strong>
                            <div className="mt-1 flex gap-2 flex-wrap">
                              <Badge variant="outline" className="text-xs">
                                event_id
                              </Badge>
                              <Badge variant="outline" className="text-xs">
                                timestamp
                              </Badge>
                              <Badge variant="outline" className="text-xs">
                                + any properties
                              </Badge>
                            </div>
                          </div>
                          <div className="text-xs text-muted-foreground">
                            • Headers are required in the first row • Timestamp should be in ISO
                            8601 format • Additional columns will be stored as event properties
                          </div>
                        </div>
                      </CardContent>
                    </Card>

                    {/* Configuration */}
                    <div className="grid grid-cols-1 gap-4">
                      <div className="flex items-center space-x-3">
                        <Label htmlFor="delimiter" className="text-sm font-medium min-w-[80px]">
                          Delimiter
                        </Label>
                        <Input
                          id="delimiter"
                          value={csvOptions.delimiter}
                          onChange={e =>
                            setCsvOptions(prev => ({ ...prev, delimiter: e.target.value }))
                          }
                          placeholder=","
                          maxLength={1}
                          className="w-20"
                        />
                      </div>

                      <div className="flex items-start space-x-3">
                        <div>
                          <Checkbox
                            id="failOnError"
                            checked={csvOptions.failOnError}
                            onCheckedChange={checked =>
                              setCsvOptions(prev => ({ ...prev, failOnError: checked === true }))
                            }
                          />
                        </div>
                        <div className="space-y-1">
                          <Label htmlFor="failOnError" className="text-sm font-medium">
                            Reject on error
                          </Label>
                          <p className="text-xs text-muted-foreground">
                            Reject the entire import if any row contains an error. Otherwise, import
                            valid rows and report errors.
                          </p>
                        </div>
                      </div>
                    </div>

                    {/* Upload Button */}
                    <div className="flex gap-2">
                      <Button
                        onClick={handleFileUpload}
                        disabled={!uploadFile || uploadMutation.isPending}
                        className="flex-1"
                        size="lg"
                      >
                        {uploadMutation.isPending ? (
                          <>
                            <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2"/>
                            Processing...
                          </>
                        ) : (
                          <>
                            <FileUpIcon className="h-4 w-4 mr-2"/>
                            Import Events
                          </>
                        )}
                      </Button>
                      <Button variant="outline" onClick={handleCloseModal}>
                        Cancel
                      </Button>
                    </div>
                  </>
                ) : (
                  <div className="space-y-4">
                    <div className="space-y-4">
                      {/* Summary */}
                      <Card>
                        <CardContent className="p-4">
                          <div className="flex items-center justify-between">
                            <div className="flex items-center gap-2">
                              {importResult.failures.length === 0 ? (
                                <CheckCircleIcon className="h-5 w-5 text-green-500"/>
                              ) : importResult.successful === 0 ? (
                                <XCircleIcon className="h-5 w-5 text-red-500"/>
                              ) : (
                                <AlertCircleIcon className="h-5 w-5 text-yellow-500"/>
                              )}
                              <div>
                                <h3 className="font-medium">
                                  {importResult.failures.length === 0
                                    ? 'Import Successful'
                                    : importResult.successful === 0
                                      ? 'Import Failed'
                                      : 'Import Partially Successful'}
                                </h3>
                                <p className="text-sm text-muted-foreground">
                                  {importResult.successful} of {importResult.totalRows} events
                                  imported
                                  {importResult.failures.length > 0 &&
                                    `, ${importResult.failures.length} failed`}
                                </p>
                              </div>
                            </div>
                          </div>
                        </CardContent>
                      </Card>

                      {/* Errors */}
                      {importResult.failures.length > 0 && (
                        <Card>
                          <CardContent className="p-4">
                            <ScrollArea className="max-h-60">
                              <div className="space-y-2">
                                {importResult.failures.map((failure, index) => (
                                  <div
                                    key={index}
                                    className="p-3 border rounded-lg bg-red-50 border-red-200 dark:bg-red-950/50 dark:border-red-800"
                                  >
                                    <div className="flex items-start justify-between">
                                      <div className="flex-1">
                                        <div className="flex items-center gap-2 text-sm font-medium mb-1">
                                          <Badge variant="destructive" className="text-xs">
                                            Row {failure.rowNumber}
                                          </Badge>
                                          {failure.eventId && (
                                            <Badge variant="outline" className="text-xs font-mono">
                                              {failure.eventId}
                                            </Badge>
                                          )}
                                        </div>
                                        <p className="text-sm text-red-700 dark:text-red-300">
                                          {failure.reason}
                                        </p>
                                      </div>
                                    </div>
                                  </div>
                                ))}
                              </div>
                            </ScrollArea>
                          </CardContent>
                        </Card>
                      )}

                      {/* Actions */}
                      <div className="flex gap-2">
                        <Button
                          variant="outline"
                          onClick={() => {
                            setImportResult(null)
                            setUploadFile(null)
                          }}
                        >
                          Import Another File
                        </Button>
                        <Button variant="ghost" onClick={handleCloseModal}>
                          Close
                        </Button>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </DialogContent>
          </Dialog>
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
