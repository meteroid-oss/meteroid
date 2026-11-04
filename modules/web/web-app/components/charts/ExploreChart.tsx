import { BarCustomLayerProps, ResponsiveBar } from '@nivo/bar'
import { linearGradientDef } from '@nivo/core'
import {
  ComputedSerie,
  CustomLayerProps,
  type Datum,
  LineSvgProps,
  ResponsiveLine,
} from '@nivo/line'
import { useTooltip } from '@nivo/tooltip'
import dayjs from 'dayjs'
import { useMemo, useRef, useState } from 'react'

import { makeAxisFormatter, makeFormatter, scaleValue } from '@/components/charts/format'
import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { Crosshair } from '@/features/dashboard/charts/Crosshair'
import { ActiveSerieLayer } from '@/features/dashboard/charts/utils'
import { lifecycleStateLabel } from '@/lib/lifecycle'
import { useTheme } from 'providers/ThemeProvider'

import type { ChartSpec, ForecastPoint } from '@/components/charts/types'

export interface ExplorePoint {
  windowStart: string
  value: number
  dimensions: { [key: string]: string }
  /** For a per-customer breakdown, the customer this series belongs to (else empty). */
  customerId?: string
}

interface ExploreChartProps {
  entry: ChartSpec
  points: ExplorePoint[]
  /** ISO-4217 reporting currency the point values are denominated in. */
  currency: string
  isLoading: boolean
  isError: boolean
  /**
   * Optional projected continuation (line shape only). Rendered as a dashed,
   * muted second series labelled "Forecast" plus a shaded confidence band, both
   * beginning after the last actual window. Values are RAW (same unit as
   * `points`); scaled here like the actuals.
   */
  forecast?: ForecastPoint[]
  /**
   * Suppress the built-in breakdown legend (and reclaim its bottom margin) when
   * an external control — the interactive segment strip — already legends the
   * series. Defaults to showing the legend.
   */
  showLegend?: boolean
  /**
   * The full, stable ordered value set to index the series palette by, so a
   * companion (the segment strip) and the chart colour a value identically even
   * when the chart's own `points` are filtered to a subset (isolation). Defaults
   * to the chart's own breakdown keys.
   */
  paletteKeys?: string[]
  /**
   * Override the value formatter used in tooltips/labels (not the compact axis). Defaults
   * to the metric unit's formatter, which rounds `count` to whole numbers — the preview
   * passes a decimal formatter so fractional usage (sums of floats, unit conversions) isn't
   * shown rounded.
   */
  valueFormatter?: (v: number) => string
}

type ChartShape = 'line' | 'area' | 'bar' | 'stacked'

// Honour the explicitly chosen shape first — the viz switcher must win, so picking
// "Line" on a broken-down metric draws multiple lines rather than silently
// collapsing to stacked bars. Only when no shape is set do we fall back to a
// sensible default per the metric's nature (a balance reads as a line, a flow as a
// bar, a breakdown as a stacked area — one path per series, cheap even for a wide range).
const resolveShape = (entry: ChartSpec): ChartShape => {
  if (entry.chart) return entry.chart
  if (entry.groupBy) return 'area'
  return entry.summary === 'latest' ? 'line' : 'bar'
}

const isLineLike = (shape: ChartShape) => shape === 'line' || shape === 'area'

// Humanise a broken-down series/axis value for display. `customer_type` carries
// the lifecycle snake_case strings, mapped to the same labels as the badge; other
// breakdowns render their raw value.
export const groupValueLabel = (groupBy: string | undefined, value: string): string =>
  groupBy === 'customer_type' ? lifecycleStateLabel(value) : value

// Theme-aware categorical palette for stacked/grouped series, built from the design
// tokens (not Nivo's category10). Sized to the server's MAX_BREAKDOWN_SERIES (20) so a
// full breakdown never wraps and repeats a colour; the first 7 are the original hues,
// so small breakdowns look unchanged. Index still wraps as a last resort.
const lightPalette = [
  '#7c3aed', '#2563eb', '#0d9488', '#d97706', '#dc2626', '#db2777', '#65a30d',
  '#0891b2', '#9333ea', '#16a34a', '#ea580c', '#e11d48', '#0284c7', '#ca8a04',
  '#4f46e5', '#059669', '#c026d3', '#a16207', '#4d7c0f', '#be123c',
]
const darkPalette = [
  '#a78bfa', '#60a5fa', '#2dd4bf', '#fbbf24', '#f87171', '#f472b6', '#a3e635',
  '#22d3ee', '#c084fc', '#4ade80', '#fb923c', '#fb7185', '#38bdf8', '#facc15',
  '#818cf8', '#34d399', '#e879f9', '#d4a373', '#bef264', '#fb6f92',
]

/** The categorical series palette, so companions (e.g. the segment strip) colour
 *  a metric's breakdown values identically to the chart. */
export const seriesPalette = (isDark: boolean): string[] => (isDark ? darkPalette : lightPalette)

/** The top-N overflow bucket's dimension value (server-side `OTHER_LABEL`). */
export const OTHER_SEGMENT = 'Other'

/** Order breakdown values alphabetically but always sink the "Other" overflow bucket
 *  to the end — so it reads last in the strip and stacks last in the chart. Both the
 *  chart and the strip must sort by this to keep colours aligned. */
export const compareSegmentKeys = (a: string, b: string): number => {
  if (a === OTHER_SEGMENT) return b === OTHER_SEGMENT ? 0 : 1
  if (b === OTHER_SEGMENT) return -1
  return a.localeCompare(b)
}

/**
 * The set of primary-breakdown values to hide, resolved identically for every render
 * path (interactive strip, dashboard tile, public embed) so they never drift:
 *  - `saved` present (incl. empty) → honour it verbatim (a mirror of what was on screen);
 *  - `saved` absent (a pre-persistence view) → apply the default, hiding only "Other";
 *  - never hide every present value (that would blank the chart) — matches the strip's guard.
 */
export const resolveHiddenSegments = (
  saved: string[] | undefined,
  present: string[]
): Set<string> => {
  const set =
    saved !== undefined
      ? new Set(saved)
      : present.includes(OTHER_SEGMENT)
        ? new Set([OTHER_SEGMENT])
        : new Set<string>()
  return present.length > 0 && present.every(v => set.has(v)) ? new Set<string>() : set
}

const lineCommonProps: LineSvgProps = {
  data: [],
  lineWidth: 2,
  animate: false,
  enableCrosshair: false,
  enableGridX: false,
  enableGridY: false,
  enableSlices: false,
  enablePoints: false,
  colors: { datum: 'color' },
}

const axisTheme = {
  // Compact tick text so a dense range stays readable.
  axis: { ticks: { text: { fill: 'hsl(var(--muted-foreground))', fontSize: 10 } } },
  grid: { line: { stroke: 'hsl(var(--border))' } },
  // Neutralise Nivo's default (white) tooltip container so the token-styled tooltip
  // below renders correctly in dark mode.
  tooltip: { container: { background: 'transparent', boxShadow: 'none', padding: 0 } },
}

// A compact, wrapping HTML legend rendered below the plot — unlike Nivo's SVG
// legend it flows onto multiple rows and lets long series names (e.g. plan names)
// wrap instead of overflowing or truncating.
const ChartLegend = ({
  items,
}: {
  items: { id: string; label: string; color: string }[]
}) => (
  <div className="flex flex-wrap gap-x-3 gap-y-0.5 px-2 pt-1.5 text-[10px] leading-tight text-muted-foreground">
    {items.map(it => (
      <span
        key={it.id}
        title={it.label}
        className="inline-flex items-start gap-1 min-w-0 max-w-[180px]"
      >
        <span
          className="mt-[3px] h-2 w-2 shrink-0 rounded-full"
          style={{ backgroundColor: it.color }}
        />
        <span className="break-words">{it.label}</span>
      </span>
    ))}
  </div>
)

type BarRow = Record<string, number | string>
type ColumnSeg = { id: string; value: number; color: string }

// Nivo point `position.x` is relative to the inner plot area (after the left
// axis). The HTML crosshair overlay is positioned from the container edge, so it
// must add the left margin back or it sits one axis-width left of the line.
const lineMargin = { top: 10, right: 30, bottom: 30, left: 60 }

export const ExploreChart = ({
  entry,
  points,
  currency,
  isLoading,
  isError,
  forecast,
  showLegend = true,
  paletteKeys,
  valueFormatter,
}: ExploreChartProps) => {
  const theme = useTheme()
  const shape = resolveShape(entry)
  const unit = entry.unit
  const palette = theme.isDarkMode ? darkPalette : lightPalette
  const lineColor = theme.isDarkMode ? '#a78bfa' : '#7c3aed'

  const fmt = useMemo(
    () => valueFormatter ?? makeFormatter(unit, currency),
    [valueFormatter, unit, currency]
  )
  const axisFmt = useMemo(() => makeAxisFormatter(unit, currency), [unit, currency])

  // Distinct breakdown keys (stable, sorted) drive the stacked series/legend.
  const groupKeys = useMemo(() => {
    if (!entry.groupBy) return []
    const set = new Set<string>()
    for (const p of points) set.add(p.dimensions[entry.groupBy] ?? 'Unattributed')
    return Array.from(set).sort(compareSegmentKeys)
  }, [points, entry.groupBy])

  // Palette index source: the caller's full value list when supplied (keeps colours
  // stable under isolation), else the keys actually present in `points`.
  const colorKeys = paletteKeys ?? groupKeys

  const forecastColor = theme.isDarkMode ? '#94a3b8' : '#94a3b8'

  // ---- Line / area: nivo time series with the Overview crosshair tooltip -----
  // A broken-down metric draws one series per breakdown value (line: overlaid
  // lines; area: stacked bands); otherwise a single series (+ optional forecast).
  const lineData = useMemo(() => {
    if (entry.groupBy) {
      const gb = entry.groupBy
      const byKey = new Map<string, { x: string; y: number; key: string }[]>()
      for (const k of groupKeys) byKey.set(k, [])
      // Ensure every series has a point at every window (0-filled) so stacked
      // areas align and lines don't break across gaps.
      const windows = Array.from(new Set(points.map(p => p.windowStart))).sort()
      const seed = new Map<string, Map<string, number>>()
      for (const k of groupKeys) seed.set(k, new Map(windows.map(w => [w, 0])))
      for (const p of points) {
        const k = p.dimensions[gb] ?? 'Unattributed'
        const row = seed.get(k)
        if (row) row.set(p.windowStart, (row.get(p.windowStart) ?? 0) + scaleValue(unit, currency, p.value))
      }
      return groupKeys.map(k => ({
        id: k,
        data: windows.map(w => ({ x: w, y: seed.get(k)!.get(w) ?? 0, key: w })),
      }))
    }

    const actual = {
      id: entry.label,
      data: points.map(p => ({
        x: p.windowStart,
        y: scaleValue(unit, currency, p.value),
        key: p.windowStart,
      })),
    }
    const series = [actual]
    if (forecast && forecast.length > 0) {
      // Bridge the dashed series from the last actual point so it visually
      // continues the line rather than floating disconnected.
      const lastActual = actual.data[actual.data.length - 1]
      const bridge = lastActual ? [{ x: String(lastActual.x), y: lastActual.y, key: 'bridge' }] : []
      series.push({
        id: 'Forecast',
        data: [
          ...bridge,
          ...forecast.map(p => ({
            x: p.windowStart,
            y: scaleValue(unit, currency, p.value),
            key: p.windowStart,
          })),
        ],
      })
    }
    return series
  }, [points, entry.label, entry.groupBy, groupKeys, unit, currency, forecast])

  // Scaled confidence band (single series only), bridged from the last actual so
  // it opens pinched on the line. Fed to the ForecastBand layer below.
  const forecastBandScaled = useMemo(() => {
    if (entry.groupBy || !forecast || forecast.length === 0) return []
    const last = [...points].sort((a, b) => a.windowStart.localeCompare(b.windowStart)).at(-1)
    const bridgeY = scaleValue(unit, currency, last?.value ?? forecast[0].value)
    const bridgeX = last?.windowStart ?? forecast[0].windowStart
    return [
      { x: bridgeX, lo: bridgeY, hi: bridgeY },
      ...forecast.map(p => ({
        x: p.windowStart,
        lo: scaleValue(unit, currency, p.lower),
        hi: scaleValue(unit, currency, p.upper),
      })),
    ]
  }, [forecast, points, entry.groupBy, unit, currency])

  const { yMin, yMax } = useMemo(() => {
    // Include the forecast band so it's fully visible, but the band is bounded
    // (damped/clamped/run-rate) so it can't crush the actual line the way the old
    // runaway projection did.
    const ys = [
      ...lineData.flatMap(s => s.data.map(d => d.y)),
      ...forecastBandScaled.flatMap(b => [b.lo, b.hi]),
    ]
    if (ys.length === 0) return { yMin: 0, yMax: 0 }
    // Single pass rather than Math.max(...ys): spreading a large array as call args
    // overflows the stack (a fine grain over a wide window is easily >100k points).
    let max = ys[0]
    let min = 0 // 0 is always a floor, matching the old Math.min(...ys, 0)
    for (const y of ys) {
      if (y > max) max = y
      if (y < min) min = y
    }
    const pad = (max - min) * 0.1
    return { yMin: min === 0 ? 0 : min - pad, yMax: max + pad }
  }, [lineData, forecastBandScaled])

  const [serie, setSerie] = useState<ComputedSerie[]>([])
  const containerRef = useRef<HTMLDivElement>(null)

  // ---- Bar / stacked: one row per window, a key per breakdown ----------------
  const barData = useMemo(() => {
    const byWindow = new Map<string, Record<string, number | string>>()
    const keys = entry.groupBy ? groupKeys : [entry.label]
    for (const p of points) {
      const w = p.windowStart
      if (!byWindow.has(w)) {
        const row: Record<string, number | string> = { window: w }
        for (const k of keys) row[k] = 0
        byWindow.set(w, row)
      }
      const k = entry.groupBy ? (p.dimensions[entry.groupBy] ?? 'Unattributed') : entry.label
      const row = byWindow.get(w)!
      row[k] = (Number(row[k]) || 0) + scaleValue(unit, currency, p.value)
    }
    return Array.from(byWindow.keys())
      .sort()
      .map(w => byWindow.get(w)!)
  }, [points, entry.groupBy, entry.label, groupKeys, unit, currency])

  const barKeys = entry.groupBy ? groupKeys : [entry.label]

  // Nivo's band (bar) axis ignores a numeric `tickValues`, so thin the labels
  // ourselves: show at most ~8 evenly-spaced dates, else they overlap unreadably.
  const barTickValues = useMemo(() => {
    const windows = barData.map(r => String(r.window))
    const maxTicks = 8
    if (windows.length <= maxTicks) return undefined
    const step = Math.ceil(windows.length / maxTicks)
    return windows.filter((_, i) => i % step === 0)
  }, [barData])

  const isEmpty = points.length === 0

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }
  if (isEmpty) {
    return <ChartNoData error={isError} />
  }

  if (isLineLike(shape)) {
    const isArea = shape === 'area'
    const multi = !!entry.groupBy
    const legendVisible = multi && showLegend
    const firstX = lineData[0]?.data[0]?.x
    // Extend the x-domain to the last forecast point when present, else the last actual.
    const allX = lineData.flatMap(s => s.data.map(d => d.x))
    const lastX = allX[allX.length - 1]
    const hasForecast = !multi && !!forecast && forecast.length > 0
    // Per-breakdown palette colour for a split series; the single-series line keeps
    // the brand colour, and the forecast continuation its muted grey.
    const colorOf = (id: string | number): string =>
      id === 'Forecast'
        ? forecastColor
        : multi
          ? palette[Math.max(0, colorKeys.indexOf(String(id))) % palette.length]
          : lineColor
    // Stacked bands for a broken-down area; a single area keeps a fixed domain.
    const yScale = (
      multi
        ? { type: 'linear', min: 'auto', max: 'auto', stacked: isArea }
        : { type: 'linear', min: yMin, max: yMax }
    ) as LineSvgProps['yScale']
    const legendItems = groupKeys.map((k, i) => ({
      id: k,
      label: groupValueLabel(entry.groupBy, k),
      color: palette[i % palette.length],
    }))
    return (
      <div className="flex h-full flex-col">
        <div className="relative min-h-0 flex-1" ref={containerRef}>
          <div className="h-0 w-0">
            <div className="chart-dotted-bg" />
          </div>
        <LineCrosshair
          serie={serie}
          containerRef={containerRef}
          format={fmt}
          marginLeft={lineMargin.left}
          labelOf={id => groupValueLabel(entry.groupBy, id)}
          showSeries={multi}
        />
        <ResponsiveLine
          {...lineCommonProps}
          theme={axisTheme}
          areaOpacity={multi ? 0.5 : 0.15}
          enableArea={isArea}
          // A single series fades out with a soft gradient; stacked breakdown bands
          // fill flat (a fading gradient would leave visible seams between stacks).
          defs={
            !multi
              ? [
                  linearGradientDef('exploreArea', [
                    { offset: 0, color: 'inherit' },
                    { offset: 100, color: 'inherit', opacity: 0 },
                  ]),
                ]
              : undefined
          }
          fill={!multi ? [{ match: (d: Datum) => d.id !== 'Forecast', id: 'exploreArea' }] : undefined}
          colors={(s: { id: string | number }) => colorOf(s.id)}
          data={lineData}
          margin={lineMargin}
          xScale={{
            type: 'time',
            format: '%Y-%m-%d',
            precision: 'day',
            useUTC: false,
            min: firstX ? new Date(firstX as string) : 'auto',
            max: lastX ? new Date(lastX as string) : 'auto',
          }}
          xFormat="time:%b %d, %Y"
          yScale={yScale}
          axisBottom={{
            tickSize: 0,
            tickPadding: 10,
            tickRotation: 0,
            format: '%b %d',
            tickValues: 5,
          }}
          axisLeft={{ tickSize: 0, tickPadding: 8, format: (v: number) => axisFmt(Number(v)) }}
          layers={[
            'grid',
            'areas',
            // Shaded confidence band under the dashed projection.
            ...(hasForecast
              ? [
                  (props: CustomLayerProps) => (
                    <ForecastBand
                      key="forecast-band"
                      xScale={props.xScale as (v: unknown) => number}
                      yScale={props.yScale as (v: unknown) => number}
                      band={forecastBandScaled}
                      color={forecastColor}
                    />
                  ),
                ]
              : []),
            // When forecasting, draw the dashed continuation ourselves so the
            // projected series is visually distinct from the solid actuals.
            hasForecast ? (props => <ForecastLines {...props} />) : 'lines',
            'axes',
            props => <ActiveSerieLayer {...props} setSerie={setSerie} />,
          ]}
        />
        {hasForecast && (
          <div className="absolute top-1 right-1 flex items-center gap-1.5 text-[10px] text-muted-foreground bg-background/70 rounded px-1.5 py-0.5">
            <svg width="16" height="2" aria-hidden>
              <line
                x1="0"
                y1="1"
                x2="16"
                y2="1"
                stroke={forecastColor}
                strokeWidth="2"
                strokeDasharray="4 3"
              />
            </svg>
            Forecast
          </div>
        )}
        </div>
        {legendVisible && <ChartLegend items={legendItems} />}
      </div>
    )
  }

  // Bar + stacked share one ResponsiveBar; a plain bar places breakdown series
  // side by side (grouped), stacked piles them; both use the breakdown palette.
  const barLegendVisible = !!entry.groupBy && showLegend
  const barLegendItems = barKeys.map((k, i) => ({
    id: k,
    label: groupValueLabel(entry.groupBy, k),
    color: palette[i % palette.length],
  }))
  return (
    <div className="flex h-full flex-col">
      <div className="relative min-h-0 flex-1">
        <ResponsiveBar
        data={barData}
        keys={barKeys}
        indexBy="window"
        groupMode={shape === 'stacked' ? 'stacked' : 'grouped'}
        animate={false}
        theme={axisTheme}
        colors={entry.groupBy ? bar => palette[Math.max(0, colorKeys.indexOf(String(bar.id))) % palette.length] : [lineColor]}
        margin={{ top: 10, right: 20, bottom: 30, left: 60 }}
        padding={0.3}
        enableLabel={false}
        enableGridX={false}
        enableGridY
        valueScale={{ type: 'linear' }}
        axisBottom={{
          tickSize: 0,
          tickPadding: 10,
          tickRotation: 0,
          format: (v: string) => dayjs(v).format('MMM D'),
          tickValues: barTickValues,
        }}
        axisLeft={{ tickSize: 0, tickPadding: 8, format: (v: number) => axisFmt(Number(v)) }}
        // Full-height hover overlay (below) handles the tooltip so the whole column is
        // hoverable — a 1px bar next to a tall one would otherwise be unhittable.
        layers={[
          'grid',
          'axes',
          'bars',
          layer => (
            <BarHoverLayer
              {...layer}
              fmt={fmt}
              showSeries={!!entry.groupBy}
              labelOf={id => groupValueLabel(entry.groupBy, id)}
            />
          ),
        ]}
        />
      </div>
      {barLegendVisible && <ChartLegend items={barLegendItems} />}
    </div>
  )
}

// Filled polygon between the forecast band's upper and lower edges, mapped through
// the chart's own scales. Sits under the dashed projection line.
const ForecastBand = ({
  xScale,
  yScale,
  band,
  color,
}: {
  xScale: (v: unknown) => number
  yScale: (v: unknown) => number
  band: { x: string; lo: number; hi: number }[]
  color: string
}) => {
  if (band.length < 2) return null
  const pts = [
    ...band.map(p => `${xScale(new Date(p.x))},${yScale(p.hi)}`),
    ...[...band].reverse().map(p => `${xScale(new Date(p.x))},${yScale(p.lo)}`),
  ]
  return <path d={`M${pts.join('L')}Z`} fill={color} fillOpacity={0.13} stroke="none" />
}

// Replacement for Nivo's default `lines` layer: draws the actual series solid and
// the "Forecast" series dashed, so the projected continuation reads as distinct.
const ForecastLines = ({ series, lineGenerator }: CustomLayerProps) => (
  <g>
    {series.map(s => {
      const d = lineGenerator(s.data.map(p => p.position)) ?? undefined
      const isForecast = s.id === 'Forecast'
      return (
        <path
          key={s.id}
          d={d ?? undefined}
          fill="none"
          stroke={s.color}
          strokeWidth={2}
          strokeDasharray={isForecast ? '5 4' : undefined}
        />
      )
    })}
  </g>
)

// A transparent full-height rect per column (spanning the band slot incl. its padding
// gaps) so hovering anywhere over a column shows its tooltip — even for a tiny bar next
// to a tall one. Drives Nivo's cursor-following tooltip with the token-styled content.
const BarHoverLayer = ({
  bars,
  innerWidth,
  innerHeight,
  fmt,
  showSeries,
  labelOf,
}: BarCustomLayerProps<BarRow> & {
  fmt: (v: number) => string
  showSeries: boolean
  labelOf: (id: string) => string
}) => {
  const { showTooltipFromEvent, hideTooltip } = useTooltip()
  const [hovered, setHovered] = useState<string | null>(null)

  const columns = useMemo(() => {
    const m = new Map<string, { center: number; index: string; segs: ColumnSeg[] }>()
    for (const b of bars) {
      const index = String(b.data.indexValue)
      const col = m.get(index) ?? { center: b.x + b.width / 2, index, segs: [] }
      if (b.data.value != null) {
        col.segs.push({ id: String(b.data.id), value: Number(b.data.value), color: b.color })
      }
      m.set(index, col)
    }
    return Array.from(m.values())
  }, [bars])

  const slot = columns.length > 0 ? innerWidth / columns.length : 0
  const active = columns.find(c => c.index === hovered)

  return (
    <g>
      {active && (
        <rect
          x={active.center - slot / 2}
          y={0}
          width={slot}
          height={innerHeight}
          style={{ fill: 'hsl(var(--muted-foreground))', fillOpacity: 0.1, pointerEvents: 'none' }}
        />
      )}
      {columns.map(c => {
        const tip = (
          <ColumnTooltip
            window={c.index}
            segs={c.segs}
            fmt={fmt}
            showSeries={showSeries}
            labelOf={labelOf}
          />
        )
        const onHover = (e: React.MouseEvent) => {
          setHovered(c.index)
          showTooltipFromEvent(tip, e)
        }
        return (
          <rect
            key={c.index}
            x={c.center - slot / 2}
            y={0}
            width={slot}
            height={innerHeight}
            fillOpacity={0}
            onMouseEnter={onHover}
            onMouseMove={onHover}
            onMouseLeave={() => {
              setHovered(null)
              hideTooltip()
            }}
          />
        )
      })}
    </g>
  )
}

// Token-styled tooltip (bg-popover / text-popover-foreground / border-border) showing
// every segment of the hovered column at once.
const ColumnTooltip = ({
  window,
  segs,
  fmt,
  showSeries,
  labelOf,
}: {
  window: string
  segs: ColumnSeg[]
  fmt: (v: number) => string
  showSeries: boolean
  labelOf: (id: string) => string
}) => (
  <div className="text-sm bg-opacity-95 bg-popover text-popover-foreground shadow-lg rounded-md py-2 px-3 border border-border">
    <div className="text-xs font-semibold mb-1">{dayjs(window).format('MMM D, YYYY')}</div>
    {segs.map(s => (
      <div key={s.id} className="flex items-center justify-between gap-3">
        <span className="flex items-center gap-1.5 text-muted-foreground">
          {showSeries && (
            <span className="h-2 w-2 rounded-full" style={{ backgroundColor: s.color }} />
          )}
          {labelOf(s.id)}
        </span>
        <span className="font-medium tabular-nums">{fmt(s.value)}</span>
      </div>
    ))}
  </div>
)

// Crosshair tooltip for the line shape, reusing the Overview Crosshair primitives
// but formatting in the explore response currency (no /100 — values are already
// scaled to major units in `lineData`).
const LineCrosshair = ({
  serie,
  containerRef,
  format,
  marginLeft,
  labelOf,
  showSeries,
}: {
  serie: ComputedSerie[]
  containerRef: React.RefObject<HTMLDivElement>
  format: (v: number) => string
  marginLeft: number
  labelOf: (id: string) => string
  showSeries: boolean
}) => {
  const tooltipRef = useRef<HTMLDivElement>(null)
  const first = serie?.[0]

  const tooltipLeft = useMemo(() => {
    const containerBox = containerRef.current?.getBoundingClientRect()
    const tooltipRect = tooltipRef.current?.getBoundingClientRect()
    if (!tooltipRect || !first || !containerBox) return -999999
    const offset = 8
    const anchor = first.data[0].position.x + marginLeft
    // Prefer the right of the crosshair, but flip to the left when that would spill
    // past the container's right edge (which clips it) — e.g. hovering the last point.
    const flip = anchor + offset + tooltipRect.width > containerBox.width
    return flip
      ? Math.max(0, anchor - offset - tooltipRect.width)
      : anchor + offset
  }, [first, containerRef, marginLeft])

  if (!serie || !serie[0]) return null

  // Keep only series whose nearest point is at serie[0]'s window (drops e.g. actuals
  // clamped to their last point over the forecast tail). Compare pixel x, not data.x:
  // the time scale parses each datum's x into its own Date, so !== never matches.
  const anchorX = first.data[0].position.x

  return (
    <>
      <Crosshair.Line style={{ left: `${serie[0].data[0].position.x + marginLeft}px` }} />
      <Crosshair.Tooltip style={{ left: tooltipLeft }} ref={tooltipRef}>
        <h3 className="text-xs font-semibold mb-2">
          {dayjs(first.data[0].data.x as string).format('MMM D, YYYY')}
        </h3>
        {serie.map(s => {
          if (s.data[0].data.y === null || s.data[0].data.y === undefined) return null
          if (s.data[0].position.x !== anchorX) return null
          return (
            <div key={s.id} className="flex items-center justify-between gap-4">
              <span className="flex items-center gap-1.5 font-semibold">
                {showSeries && (
                  <span className="h-2 w-2 rounded-full" style={{ backgroundColor: s.color }} />
                )}
                {labelOf(String(s.id))}
              </span>
              <span className="tabular-nums">{format(Number(s.data[0].data.y))}</span>
            </div>
          )
        })}
      </Crosshair.Tooltip>
    </>
  )
}
