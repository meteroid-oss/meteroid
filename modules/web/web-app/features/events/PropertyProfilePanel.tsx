import { Skeleton } from '@md/ui'
import { HashIcon, TypeIcon } from 'lucide-react'
import { ReactNode, useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { profileEventProperties } from '@/rpc/api/eventinsights/v1/eventinsights-EventInsightsService_connectquery'

import type { PropertyProfile } from '@/rpc/api/eventinsights/v1/eventinsights_pb'

export interface PropertyProfilePanelProps {
  /** Event code whose raw-event properties to profile. Empty disables the query. */
  eventCode: string
  /** Inclusive lower / exclusive upper time bounds. */
  from: Date
  to: Date
  className?: string
}

const isoDate = (d: Date) => d.toISOString().slice(0, 10)

const fmtInt = (v: number) => v.toLocaleString(undefined, { maximumFractionDigits: 0 })
const fmtNum = (v: number) =>
  Math.abs(v) >= 1000
    ? v.toLocaleString(undefined, { notation: 'compact', maximumFractionDigits: 1 })
    : v.toLocaleString(undefined, { maximumFractionDigits: 2 })
const pct = (ratio: number) => `${Math.round(ratio * 100)}%`

/**
 * Statistical profile of an event source's raw-event properties over a time range:
 * per property key its coverage (the missing-data signal), cardinality, and either a
 * numeric distribution (min/p50/avg/p95/max/sum) or the top values as a proportion
 * bar list. Drives the metric-preview schema tab and the Events-page profiler. Reads
 * the same `raw_events_v2` a metric over this code would aggregate.
 */
export const PropertyProfilePanel = ({ eventCode, from, to, className }: PropertyProfilePanelProps) => {
  const code = eventCode.trim()
  const request = useMemo(
    () => ({ eventCode: code, from: isoDate(from), to: isoDate(to), topValues: 6, limit: 100 }),
    [code, from, to]
  )
  const { data, isLoading, error } = useQuery(profileEventProperties, request, {
    enabled: code.length > 0,
  })

  const properties = data?.properties ?? []
  const totalEvents = data ? Number(data.totalEvents) : 0

  if (code.length === 0)
    return (
      <PanelMessage>
        Enter an event code to profile the properties your events carry.
      </PanelMessage>
    )
  if (isLoading) return <Skeleton height={280} />
  if (error) return <PanelMessage>Couldn’t profile properties for this event source.</PanelMessage>
  if (properties.length === 0)
    return <PanelMessage>No events found for this code in the selected range.</PanelMessage>

  return (
    <div className={className}>
      <p className="text-[11px] text-muted-foreground mb-3">
        {fmtInt(totalEvents)} events · {properties.length} propert
        {properties.length === 1 ? 'y' : 'ies'} in the selected range
      </p>
      <div className="space-y-2">
        {properties.map(p => (
          <PropertyCard key={p.key} profile={p} />
        ))}
      </div>
    </div>
  )
}

const PropertyCard = ({ profile }: { profile: PropertyProfile }) => {
  const isNumeric = !!profile.numeric
  const eventCount = Number(profile.eventCount)
  const cardinality = Number(profile.cardinality)

  return (
    <div className="border border-border rounded-lg p-3">
      <div className="flex items-center gap-2 mb-2">
        <span className="inline-flex items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
          {isNumeric ? <HashIcon className="h-3 w-3" /> : <TypeIcon className="h-3 w-3" />}
          {isNumeric ? 'number' : 'text'}
        </span>
        <span className="font-mono text-xs font-medium truncate">{profile.key}</span>
        <span className="ml-auto text-[11px] text-muted-foreground whitespace-nowrap">
          {fmtInt(cardinality)} distinct · {fmtInt(eventCount)} events
        </span>
      </div>

      {/* Coverage — the share of events that actually carry this key. */}
      <div className="flex items-center gap-2 mb-2">
        <div className="h-1.5 flex-1 rounded-full bg-muted overflow-hidden">
          <div
            className="h-full rounded-full bg-brand"
            style={{ width: pct(Math.min(1, profile.coverageRatio)) }}
          />
        </div>
        <span className="text-[11px] tabular-nums text-muted-foreground w-24 text-right">
          {pct(profile.coverageRatio)} coverage
        </span>
      </div>

      {isNumeric && profile.numeric ? (
        <NumericStats stats={profile.numeric} />
      ) : (
        <TopValues values={profile.topValues} eventCount={eventCount} />
      )}
    </div>
  )
}

const NumericStats = ({
  stats,
}: {
  stats: NonNullable<PropertyProfile['numeric']>
}) => {
  const cells: [string, number][] = [
    ['min', stats.min],
    ['p50', stats.p50],
    ['avg', stats.avg],
    ['p95', stats.p95],
    ['max', stats.max],
    ['sum', stats.sum],
  ]
  return (
    <div className="grid grid-cols-3 sm:grid-cols-6 gap-1.5">
      {cells.map(([label, value]) => (
        <div key={label} className="rounded-md bg-muted/50 px-2 py-1.5 text-center">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</div>
          <div className="text-xs font-medium tabular-nums">{fmtNum(value)}</div>
        </div>
      ))}
    </div>
  )
}

const TopValues = ({
  values,
  eventCount,
}: {
  values: PropertyProfile['topValues']
  eventCount: number
}) => {
  if (values.length === 0)
    return <p className="text-[11px] text-muted-foreground">No non-empty values.</p>

  const max = values.reduce((m, v) => Math.max(m, Number(v.count)), 0)
  return (
    <div className="space-y-1">
      {values.map(v => {
        const count = Number(v.count)
        const share = eventCount > 0 ? count / eventCount : 0
        return (
          <div key={v.value} className="flex items-center gap-2">
            <span className="font-mono text-[11px] truncate w-32 shrink-0" title={v.value}>
              {v.value}
            </span>
            <div className="h-1.5 flex-1 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full rounded-full bg-brand/70"
                style={{ width: `${max > 0 ? (count / max) * 100 : 0}%` }}
              />
            </div>
            <span className="text-[11px] tabular-nums text-muted-foreground w-20 text-right whitespace-nowrap">
              {fmtInt(count)} · {pct(share)}
            </span>
          </div>
        )
      })}
    </div>
  )
}

const PanelMessage = ({ children }: { children: ReactNode }) => (
  <div className="min-h-[200px] flex items-center justify-center text-center text-xs text-muted-foreground border border-dashed border-border rounded-lg px-4">
    {children}
  </div>
)
