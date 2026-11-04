import { create, type MessageInitShape } from '@bufbuild/protobuf'
import { timestampDate, timestampFromDate } from '@bufbuild/protobuf/wkt'
import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { ExternalLink, EyeIcon } from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import {
  compareSegmentKeys,
  ExploreChart,
  type ExplorePoint,
  resolveHiddenSegments,
} from '@/components/charts/ExploreChart'
import { SegmentSummary } from '@/components/charts/SegmentSummary'
import { DataTableViewOptions } from '@/components/table/DataTableViewOptions'
import { useModule } from '@/edition'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import {
  EventCustomerLink,
  EventDetailsDialog,
  stringifyProperties,
} from '@/features/events/EventDetails'
import { PropertyProfilePanel } from '@/features/events/PropertyProfilePanel'
import { isAdditiveAggregation } from '@/features/productCatalog/metrics/aggregation'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useTableDensity, useTablePreferences } from '@/hooks/useTablePreferences'
import { useQuery } from '@/lib/connectrpc'
import {
  Aggregation_AggregationType,
  MetricFilter_Operator,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { queryMetricUsage } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'
import {
  QueryMetricUsageRequest_Breakdown,
  QueryMetricUsageRequest_Grain,
  type MetricDefinitionSchema,
} from '@/rpc/api/eventinsights/v1/eventinsights_pb'
import { searchEvents } from '@/rpc/api/events/v1/events-EventsService_connectquery'
import {
  EventPropertyFilter_FilterOp,
  SearchEventsRequestSchema,
  SearchEventsRequest_SortOrder,
  type EventSummary,
} from '@/rpc/api/events/v1/events_pb'
import { useTheme } from 'providers/ThemeProvider'

import type { ChartSpec } from '@/components/charts/types'
import type {
  AggregationSchema,
  SegmentationMatrixSchema,
} from '@/rpc/api/billablemetrics/v1/models_pb'

export type MetricPreviewTab = 'chart' | 'totals' | 'schema' | 'raw'

const TAB_LABEL: Record<MetricPreviewTab, string> = {
  chart: 'Chart',
  totals: 'Totals',
  schema: 'Properties',
  raw: 'Raw events',
}

type RangePreset = '7d' | '30d' | '90d'

const RANGES: Record<RangePreset, { label: string; days: number }> = {
  '7d': { label: 'Last 7 days', days: 7 },
  '30d': { label: 'Last 30 days', days: 30 },
  '90d': { label: 'Last 90 days', days: 90 },
}

const RAW_EVENTS_LIMIT = 10

/** Group-by value for the per-customer split; also the dimension key the server labels it with. */
const CUSTOMER_BREAKDOWN = 'customer'
/** The overflow bucket the server folds the long tail into. */
const OTHER_LABEL = 'Other'
/** Group-by value meaning "one overall total, no split". */
const NO_GROUP = 'none'
/** How many groups to rank; the rest land in the server's "Other" bucket. */
const TOP_N = 8

/** A filter as the preview consumes it (the proto `MetricFilter` shape). */
export type PreviewFilter = { property: string; op: MetricFilter_Operator; values: string[] }

/** The four metric-filter operators collapse to the events search's include/exclude. */
const FILTER_OP_TO_EVENT_OP: Record<MetricFilter_Operator, EventPropertyFilter_FilterOp> = {
  [MetricFilter_Operator.EQUAL]: EventPropertyFilter_FilterOp.IN,
  [MetricFilter_Operator.IN]: EventPropertyFilter_FilterOp.IN,
  [MetricFilter_Operator.NOT_EQUAL]: EventPropertyFilter_FilterOp.NOT_IN,
  [MetricFilter_Operator.NOT_IN]: EventPropertyFilter_FilterOp.NOT_IN,
}

type SegmentationMatrixInit = MessageInitShape<typeof SegmentationMatrixSchema>

export interface MetricPreviewProps {
  /** Event code the metric aggregates. Empty disables the whole preview. */
  code: string
  aggregation?: MessageInitShape<typeof AggregationSchema>
  segmentationMatrix?: MessageInitShape<typeof SegmentationMatrixSchema>
  /** Pre-aggregation filters, applied so the preview matches what billing would meter. */
  filters?: PreviewFilter[]
  usageGroupKey?: string
  /** The persisted metric's catalog key (`bm:<id>`); when set and the intelligence module is
   * present, chart/totals deep-link into Explore. Absent for draft (create) metrics. */
  exploreLinkKey?: string
  /** Which tabs to show, in order. Defaults to chart + totals + raw events. */
  tabs?: MetricPreviewTab[]
  className?: string
}

const fmtUsage = (v: number) => v.toLocaleString(undefined, { maximumFractionDigits: 2 })

/**
 * Live preview of a billable metric over the tenant's raw events, driven by a draft
 * (create/edit) or existing (detail) definition — nothing is persisted. The definition is
 * sent as a draft to EventInsightsService.QueryMetricUsage, which meters it as billing would
 * (filters, segmentation matrix, unit conversion). A segmented switch flips between the
 * usage chart, the period totals, the event properties and the raw events feeding the metric.
 */
export const MetricPreview = ({
  code,
  aggregation,
  segmentationMatrix,
  filters,
  usageGroupKey,
  exploreLinkKey,
  tabs = ['chart', 'totals', 'schema', 'raw'],
  className,
}: MetricPreviewProps) => {
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const hasIntelligence = useModule('intelligence') !== undefined

  const [tab, setTab] = useState<MetricPreviewTab>(tabs[0] ?? 'chart')
  const [range, setRange] = useState<RangePreset>('30d')
  const [groupBy, setGroupBy] = useState<string>(NO_GROUP)
  const [customerId, setCustomerId] = useState<string | undefined>()
  const [selectedEvent, setSelectedEvent] = useState<EventSummary | null>(null)
  // Raw-events tab: restrict to events the metric would actually count. On by default; the
  // toggle only surfaces when the metric has filters (otherwise it's a no-op).
  const [onlyMatching, setOnlyMatching] = useState(true)
  // Segments toggled off in the chart. `undefined` = default (hide only "Other");
  // any explicit array (incl. empty) mirrors the user's strip interactions verbatim.
  const [hiddenSegments, setHiddenSegments] = useState<string[] | undefined>(undefined)
  const theme = useTheme()

  // Changing the breakdown resets the strip back to its default (hide "Other").
  const onGroupByChange = (value: string) => {
    setGroupBy(value)
    setHiddenSegments(undefined)
  }

  const activeTab = tabs.includes(tab) ? tab : (tabs[0] ?? 'chart')
  const trimmedCode = code.trim()
  const aggregationType = aggregation?.aggregationType
  // Every aggregation except COUNT aggregates a property, so the engine requires a
  // value_property; don't fire the preview (which would 500) until the key is set.
  const needsValueProperty =
    aggregationType != null && aggregationType !== Aggregation_AggregationType.COUNT
  const hasValueProperty = !!aggregation?.aggregationKey?.trim()
  const hasDefinition =
    trimmedCode.length > 0 &&
    aggregationType != null &&
    (!needsValueProperty || hasValueProperty)

  const { from, to } = useMemo(() => {
    const end = new Date()
    const start = new Date(end)
    start.setUTCDate(start.getUTCDate() - RANGES[range].days)
    return { from: start, to: end }
  }, [range])

  // Only complete filters (a property and ≥1 value) narrow the preview; half-typed rows
  // are ignored so the chart doesn't error mid-edit.
  const completeFilters = useMemo(
    () =>
      (filters ?? [])
        .filter(f => f.property.trim().length > 0 && f.values.length > 0)
        .map(f => ({ property: f.property, op: f.op, values: f.values })),
    [filters]
  )
  const filtersKey = JSON.stringify(completeFilters)

  // Group-by options: no grouping (default total), Customer, the metric's usage group
  // key, and any segmentation dimensions.
  const segmentationKeys = useMemo(() => segmentationDimensionKeys(segmentationMatrix), [
    segmentationMatrix,
  ])
  const groupByOptions = useMemo(() => {
    const opts: { value: string; label: string }[] = [
      { value: NO_GROUP, label: 'No grouping' },
      { value: CUSTOMER_BREAKDOWN, label: 'Customer' },
    ]
    if (usageGroupKey) opts.push({ value: usageGroupKey, label: usageGroupKey })
    for (const key of segmentationKeys) opts.push({ value: key, label: key })
    return opts.filter((o, i) => opts.findIndex(x => x.value === o.value) === i)
  }, [usageGroupKey, segmentationKeys])

  const activeGroupBy = groupByOptions.some(o => o.value === groupBy) ? groupBy : NO_GROUP
  const breakdown = activeGroupBy === NO_GROUP ? undefined : activeGroupBy

  // Rebuilt as plain init objects so the JSON only changes when the definition does (the
  // create form hands new objects on every render, and saved metrics carry $typeName).
  const draftJson = hasDefinition
    ? JSON.stringify({
        code: trimmedCode,
        aggregation: {
          aggregationType,
          aggregationKey: aggregation?.aggregationKey || undefined,
          unitConversion: aggregation?.unitConversion?.factor
            ? { factor: aggregation.unitConversion.factor }
            : undefined,
        },
        segmentationMatrix: completeSegmentationMatrix(segmentationMatrix),
        filters: completeFilters,
      } satisfies MessageInitShape<typeof MetricDefinitionSchema>)
    : ''

  // Debounce on the definition alone so typing the code/aggregation in the create
  // form doesn't fire a query per keystroke; discrete selects (group-by, range,
  // customer) apply immediately.
  const settledJson = useDebounceValue(draftJson, 400)
  const draft = useMemo(
    () =>
      settledJson
        ? (JSON.parse(settledJson) as MessageInitShape<typeof MetricDefinitionSchema>)
        : undefined,
    [settledJson]
  )
  const settled = settledJson === draftJson

  const usageRequest = (grain: QueryMetricUsageRequest_Grain) => ({
    metric: { case: 'draft' as const, value: draft ?? {} },
    from: timestampFromDate(from),
    to: timestampFromDate(to),
    grain,
    breakdown: !breakdown
      ? QueryMetricUsageRequest_Breakdown.NONE
      : breakdown === CUSTOMER_BREAKDOWN
        ? QueryMetricUsageRequest_Breakdown.CUSTOMER
        : QueryMetricUsageRequest_Breakdown.PROPERTY,
    breakdownProperty: breakdown && breakdown !== CUSTOMER_BREAKDOWN ? breakdown : '',
    topN: breakdown ? TOP_N : 0,
    customerIds: customerId ? [customerId] : [],
  })

  const chartQuery = useQuery(queryMetricUsage, usageRequest(QueryMetricUsageRequest_Grain.DAY), {
    enabled: !!draft && settled && activeTab === 'chart',
  })
  const totalsQuery = useQuery(
    queryMetricUsage,
    usageRequest(QueryMetricUsageRequest_Grain.TOTAL),
    { enabled: !!draft && settled && activeTab === 'totals' }
  )

  // The metric's filters in the events-search shape (already collapsed to include/exclude).
  const rawEventFilters = useMemo(
    () =>
      onlyMatching
        ? completeFilters.map(f => ({
            property: f.property,
            values: f.values,
            op: FILTER_OP_TO_EVENT_OP[f.op],
          }))
        : [],
    // filtersKey captures completeFilters' contents for memoisation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [onlyMatching, filtersKey]
  )

  const rawEventsRequest = useMemo(
    () =>
      create(SearchEventsRequestSchema, {
        from: timestampFromDate(from),
        to: timestampFromDate(to),
        limit: RAW_EVENTS_LIMIT,
        offset: 0,
        eventCodes: trimmedCode ? [trimmedCode] : [],
        customerIds: customerId ? [customerId] : [],
        filters: rawEventFilters,
        sortOrder: SearchEventsRequest_SortOrder.TIMESTAMP_DESC,
      }),
    [from, to, trimmedCode, customerId, rawEventFilters]
  )

  const rawEventsQuery = useQuery(searchEvents, rawEventsRequest, {
    enabled: hasDefinition && activeTab === 'raw',
  })

  const chartPoints: ExplorePoint[] = useMemo(
    () => chartQuery.data?.points ?? [],
    [chartQuery.data?.points]
  )

  // The full, sorted breakdown value set ("Other" sunk last) — indexes the palette so
  // colours stay put as segments are toggled off, and feeds the interactive strip.
  const segmentKeys = useMemo(() => {
    if (!breakdown) return undefined
    return Array.from(
      new Set(chartPoints.map(p => p.dimensions[breakdown] ?? 'Unattributed'))
    ).sort(compareSegmentKeys)
  }, [breakdown, chartPoints])

  const hidden = useMemo(
    () => resolveHiddenSegments(hiddenSegments, segmentKeys ?? []),
    [hiddenSegments, segmentKeys]
  )

  const visiblePoints = useMemo(() => {
    if (!breakdown || hidden.size === 0) return chartPoints
    return chartPoints.filter(p => !hidden.has(p.dimensions[breakdown] ?? 'Unattributed'))
  }, [breakdown, hidden, chartPoints])

  const toggleSegment = (value: string) => {
    const next = new Set(hidden)
    next.has(value) ? next.delete(value) : next.add(value)
    setHiddenSegments(Array.from(next))
  }
  const isolateSegment = (value: string) => {
    const others = (segmentKeys ?? []).filter(k => k !== value)
    const alreadyIsolated = others.length > 0 && others.every(k => hidden.has(k)) && !hidden.has(value)
    setHiddenSegments(alreadyIsolated ? [] : others)
  }
  const showAllSegments = () => setHiddenSegments([])

  const totalsRows: TotalsRow[] = useMemo(() => {
    const pts = totalsQuery.data?.points ?? []
    return pts
      .map(p => {
        const label = breakdown ? (p.dimensions[breakdown] ?? OTHER_LABEL) : 'All events'
        return {
          label,
          value: p.value,
          customerId: p.customerId || undefined,
          isOther: p.other,
        }
      })
      .sort((a, b) => {
        if (a.isOther !== b.isOther) return a.isOther ? 1 : -1
        return b.value - a.value
      })
  }, [totalsQuery.data?.points, breakdown])

  const rawEvents = rawEventsQuery.data?.events ?? []

  const chartEntry: ChartSpec = {
    label: 'Usage',
    unit: 'count',
    summary: 'sum',
    groupBy: breakdown,
    chart: breakdown ? 'stacked' : 'bar',
  }

  const exploreLink =
    exploreLinkKey && hasIntelligence && !isExpress
      ? `${basePath}/analytics/reports/insights/${exploreLinkKey}${
          breakdown ? `?split=${breakdown}` : ''
        }`
      : undefined

  return (
    <div className={className}>
      <div className="flex items-center justify-between gap-2 mb-4 flex-wrap">
        {/* Segmented switch — mirrors the analytics visualization switcher. */}
        <div className="inline-flex rounded-md border border-border overflow-hidden">
          {tabs.map(t => (
            <button
              key={t}
              type="button"
              onClick={() => setTab(t)}
              className={`text-xs px-2.5 py-1.5 border-r border-border last:border-r-0 ${
                activeTab === t
                  ? 'bg-muted text-foreground font-medium'
                  : 'text-muted-foreground hover:bg-muted/50'
              }`}
            >
              {TAB_LABEL[t]}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          {activeTab === 'raw' && completeFilters.length > 0 && (
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
              Only counted
              <Switch checked={onlyMatching} onCheckedChange={setOnlyMatching} />
            </label>
          )}
          {(activeTab === 'chart' || activeTab === 'totals') && groupByOptions.length > 1 && (
            <Select value={activeGroupBy} onValueChange={onGroupByChange}>
              <SelectTrigger className="w-[170px] h-7 text-xs">
                <SelectValue placeholder="Group by" />
              </SelectTrigger>
              <SelectContent>
                {groupByOptions.map(o => (
                  <SelectItem key={o.value} value={o.value}>
                    {o.value === NO_GROUP ? o.label : `Group by ${o.label}`}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          {activeTab !== 'schema' && (
            <CustomerSelect
              value={customerId}
              onChange={id => setCustomerId(id || undefined)}
              placeholder="All customers"
              clearable
              triggerClassName="h-7 text-xs w-[150px]"
            />
          )}
          <Select value={range} onValueChange={v => setRange(v as RangePreset)}>
            <SelectTrigger className="w-[150px] h-7 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(Object.keys(RANGES) as RangePreset[]).map(key => (
                <SelectItem key={key} value={key}>
                  {RANGES[key].label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {exploreLink && (activeTab === 'chart' || activeTab === 'totals') && (
            <Button variant="secondary" size="sm" className="h-7 mt-0 text-xs" asChild>
              <Link to={exploreLink}>
                <ExternalLink size={14} className="mr-1" />
                Explore
              </Link>
            </Button>
          )}
        </div>
      </div>

      {activeTab === 'schema' ? (
        // Only needs the event code — it's the tool for discovering which property to
        // aggregate, so it stays available before the rest of the definition is set.
        trimmedCode.length > 0 ? (
          <PropertyProfilePanel eventCode={trimmedCode} from={from} to={to} />
        ) : (
          <div className="h-[240px] flex items-center justify-center text-center text-xs text-muted-foreground border border-dashed border-border rounded-lg px-4">
            Enter an event code to profile the properties your events carry.
          </div>
        )
      ) : !hasDefinition ? (
        <div className="h-[240px] flex items-center justify-center text-center text-xs text-muted-foreground border border-dashed border-border rounded-lg px-4">
          {trimmedCode.length > 0 && needsValueProperty && !hasValueProperty
            ? 'Set the aggregation key (the property to aggregate) to preview usage.'
            : 'Enter an event code and aggregation to preview usage from your events.'}
        </div>
      ) : activeTab === 'chart' ? (
        <div>
          <div className="h-[260px] relative">
            <ExploreChart
              entry={chartEntry}
              points={visiblePoints}
              currency="USD"
              isLoading={chartQuery.isLoading}
              isError={!!chartQuery.error}
              paletteKeys={breakdown ? segmentKeys : undefined}
              showLegend={false}
              // Usage is often fractional (float sums, unit conversion); don't round it away.
              valueFormatter={fmtUsage}
            />
          </div>
          {breakdown && chartPoints.length > 0 && (
            <SegmentSummary
              points={chartPoints}
              groupBy={breakdown}
              unit="count"
              currency="USD"
              summary="sum"
              isDark={theme.isDarkMode}
              hidden={hidden}
              onToggle={toggleSegment}
              onIsolate={isolateSegment}
              onShowAll={showAllSegments}
            />
          )}
        </div>
      ) : activeTab === 'totals' ? (
        <TotalsTable
          isLoading={totalsQuery.isLoading}
          isError={!!totalsQuery.error}
          groupLabel={groupByOptions.find(o => o.value === activeGroupBy)?.label ?? 'Group'}
          isGrouped={!!breakdown}
          showShare={!!breakdown && isAdditiveAggregation(aggregationType)}
          rows={totalsRows}
          basePath={basePath}
        />
      ) : (
        <RawEventsTable
          isLoading={rawEventsQuery.isLoading}
          isError={!!rawEventsQuery.error}
          events={rawEvents}
          limit={RAW_EVENTS_LIMIT}
          onInspect={setSelectedEvent}
        />
      )}

      <EventDetailsDialog event={selectedEvent} onClose={() => setSelectedEvent(null)} />
    </div>
  )
}

interface TotalsRow {
  label: string
  value: number
  customerId?: string
  isOther: boolean
}

const TotalsTable = ({
  isLoading,
  isError,
  groupLabel,
  isGrouped,
  showShare,
  rows,
  basePath,
}: {
  isLoading: boolean
  isError: boolean
  groupLabel: string
  isGrouped: boolean
  showShare: boolean
  rows: TotalsRow[]
  basePath: string
}) => {
  const total = useMemo(() => rows.reduce((acc, r) => acc + r.value, 0), [rows])

  if (isLoading) return <Skeleton height={200} />
  if (isError) return <PreviewMessage>Couldn’t load the totals for this metric.</PreviewMessage>
  if (rows.length === 0) return <PreviewMessage>No usage in the selected range.</PreviewMessage>

  return (
    <div className="border border-border rounded-lg overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{isGrouped ? groupLabel : ''}</TableHead>
            <TableHead className="text-right">Usage</TableHead>
            {showShare && <TableHead className="text-right w-16">Share</TableHead>}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, i) => {
            const share = total > 0 ? (row.value / total) * 100 : 0
            return (
              <TableRow key={i}>
                <TableCell className={row.isOther ? 'text-muted-foreground italic' : ''}>
                  {row.customerId ? (
                    <Link
                      to={`${basePath}/customers/${row.customerId}`}
                      className="text-brand hover:underline"
                    >
                      {row.label}
                    </Link>
                  ) : isGrouped ? (
                    row.label
                  ) : (
                    <span className="text-muted-foreground">{row.label}</span>
                  )}
                </TableCell>
                <TableCell className="text-right font-medium tabular-nums">
                  {fmtUsage(row.value)}
                </TableCell>
                {showShare && (
                  <TableCell className="text-right tabular-nums text-muted-foreground">
                    {share.toFixed(0)}%
                  </TableCell>
                )}
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

const RawEventsTable = ({
  isLoading,
  isError,
  events,
  limit,
  onInspect,
}: {
  isLoading: boolean
  isError: boolean
  events: EventSummary[]
  limit: number
  onInspect: (event: EventSummary) => void
}) => {
  const { density, setDensity } = useTableDensity()
  // Column visibility persists per-user (same mechanism as the customers/subscriptions tables),
  // so hiding e.g. Event ID + Timestamp to focus on Properties sticks across sessions.
  const { columnVisibility, setColumnVisibility, reset } = useTablePreferences(
    'metric-preview-events'
  )

  const columns = useMemo<ColumnDef<EventSummary>[]>(
    () => [
      {
        id: 'Event ID',
        header: 'Event ID',
        cell: ({ row }) => <span className="font-mono text-xs">{row.original.id}</span>,
      },
      {
        id: 'Timestamp',
        header: 'Timestamp',
        cell: ({ row }) => (
          <span className="whitespace-nowrap text-xs">
            {row.original.timestamp ? timestampDate(row.original.timestamp).toLocaleString() : '—'}
          </span>
        ),
      },
      {
        id: 'Customer',
        header: 'Customer',
        cell: ({ row }) => <EventCustomerLink event={row.original} />,
      },
      {
        id: 'Properties',
        header: 'Properties',
        cell: ({ row }) => {
          const props = stringifyProperties(row.original.properties)
          return (
            <span className="block max-w-[360px] truncate text-xs text-muted-foreground">
              {props.length > 2 ? props : '—'}
            </span>
          )
        },
      },
      {
        id: 'actions',
        header: '',
        enableHiding: false,
        cell: ({ row }) => (
          <Button variant="ghost" size="sm" onClick={() => onInspect(row.original)}>
            <EyeIcon className="h-4 w-4" />
          </Button>
        ),
      },
    ],
    [onInspect]
  )

  const table = useReactTable({
    data: events,
    columns,
    state: { columnVisibility },
    onColumnVisibilityChange: setColumnVisibility,
    getCoreRowModel: getCoreRowModel(),
  })

  if (isLoading) return <Skeleton height={200} />
  if (isError) return <PreviewMessage>Couldn’t load events for this metric.</PreviewMessage>
  if (events.length === 0)
    return <PreviewMessage>No events found for this code in the selected range.</PreviewMessage>

  const cellPad = density === 'compact' ? 'py-1.5' : undefined

  return (
    <div className="space-y-2">
      <div className="flex justify-end">
        <DataTableViewOptions
          table={table}
          density={density}
          onDensityChange={setDensity}
          onReset={reset}
        />
      </div>
      <div className="border border-border rounded-lg overflow-x-auto">
        <Table className="min-w-[560px]">
          <TableHeader>
            {table.getHeaderGroups().map(group => (
              <TableRow key={group.id}>
                {group.headers.map(header => (
                  <TableHead
                    key={header.id}
                    className={header.column.id === 'actions' ? 'w-4' : undefined}
                  >
                    {header.isPlaceholder
                      ? null
                      : flexRender(header.column.columnDef.header, header.getContext())}
                  </TableHead>
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {table.getRowModel().rows.map(row => (
              <TableRow key={row.id}>
                {row.getVisibleCells().map(cell => (
                  <TableCell key={cell.id} className={cellPad}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {events.length >= limit && (
        <p className="text-[11px] text-muted-foreground">
          Showing the latest {limit} events. See the Events page for the full stream.
        </p>
      )}
    </div>
  )
}

const PreviewMessage = ({ children }: { children: ReactNode }) => (
  <div className="h-[200px] flex items-center justify-center text-center text-xs text-muted-foreground">
    {children}
  </div>
)

/** Pull the dimension keys out of a segmentation matrix (single/double/linked) so
 * they can be offered as group-by options in the totals view. */
function segmentationDimensionKeys(
  segmentationMatrix?: MessageInitShape<typeof SegmentationMatrixSchema>
): string[] {
  const matrix = segmentationMatrix?.matrix as
    | { case?: string; value?: Record<string, unknown> }
    | undefined
  if (!matrix?.case) return []
  const value = matrix.value ?? {}
  const keys: (string | undefined)[] =
    matrix.case === 'single'
      ? [(value.dimension as { key?: string } | undefined)?.key]
      : matrix.case === 'double'
        ? [
            (value.dimension1 as { key?: string } | undefined)?.key,
            (value.dimension2 as { key?: string } | undefined)?.key,
          ]
        : matrix.case === 'linked'
          ? [value.dimensionKey as string | undefined, value.linkedDimensionKey as string | undefined]
          : []
  return keys.filter((k): k is string => !!k)
}

const isCompleteDimension = (d?: { key?: string; values?: string[] }) =>
  !!d?.key?.trim() && (d.values?.length ?? 0) > 0

/** The matrix as a plain init when every dimension is filled in, else undefined: like a
 * half-typed filter, a matrix still being edited doesn't narrow the preview. */
function completeSegmentationMatrix(
  segmentationMatrix?: SegmentationMatrixInit
): SegmentationMatrixInit | undefined {
  const matrix = segmentationMatrix?.matrix
  switch (matrix?.case) {
    case 'single': {
      const d = matrix.value.dimension
      return d && isCompleteDimension(d)
        ? { matrix: { case: 'single', value: { dimension: { key: d.key, values: d.values } } } }
        : undefined
    }
    case 'double': {
      const { dimension1: d1, dimension2: d2 } = matrix.value
      return d1 && d2 && isCompleteDimension(d1) && isCompleteDimension(d2)
        ? {
            matrix: {
              case: 'double',
              value: {
                dimension1: { key: d1.key, values: d1.values },
                dimension2: { key: d2.key, values: d2.values },
              },
            },
          }
        : undefined
    }
    case 'linked': {
      const { dimensionKey, linkedDimensionKey } = matrix.value
      const values = Object.entries(matrix.value.values ?? {}).filter(
        ([key, v]) => key.trim().length > 0 && (v.values?.length ?? 0) > 0
      )
      return dimensionKey?.trim() && linkedDimensionKey?.trim() && values.length > 0
        ? {
            matrix: {
              case: 'linked',
              value: {
                dimensionKey,
                linkedDimensionKey,
                values: Object.fromEntries(values.map(([key, v]) => [key, { values: v.values }])),
              },
            },
          }
        : undefined
    }
    default:
      return undefined
  }
}
