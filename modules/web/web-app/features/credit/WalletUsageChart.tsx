import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Skeleton } from '@md/ui'
import { ResponsiveBar, type BarTooltipProps } from '@nivo/bar'
import { useState } from 'react'

import { formatWalletAmount } from '@/features/credit/format'
import {
  OTHER_SERIES,
  seriesColorOrder,
  walletDailyUsageInput,
  walletUsageChartData,
} from '@/features/credit/usageChartData'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useQuery } from '@/lib/connectrpc'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { listWalletDailyUsageCost } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

import type { UsageRange } from '@/features/credit/usageChartData'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

const RANGE_LABEL: Record<UsageRange, string> = { current: 'This period', last: 'Last period' }

const MAX_SERIES = 5

// Categorical slots validated for color-vision deficiency, light and dark, in this order.
const LIGHT_SERIES = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4']
const DARK_SERIES = ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181']
const OTHER_COLOR = { light: '#9a9993', dark: '#6f6e69' }

const chartTheme = {
  text: { fill: 'hsl(var(--muted-foreground))', fontSize: 10 },
  axis: { ticks: { text: { fill: 'hsl(var(--muted-foreground))', fontSize: 10 } } },
  grid: { line: { stroke: 'hsl(var(--border))', strokeWidth: 1 } },
  tooltip: { container: { background: 'transparent', boxShadow: 'none', padding: 0 } },
}

/**
 * The wallet's usage per day over a billing period, stacked by the metric that consumed it, and its
 * top consumers. Each day is what it adds to the period's price, as the server prices it: the bars
 * match the invoice unless its lines were edited or usage arrived after it was finalized.
 */
export const WalletUsageChart = ({
  wallet,
  scope,
}: {
  wallet: Wallet
  /** The subscription whose usage is shown; every subscription's when unset. */
  scope?: string
}) => {
  const [range, setRange] = useState<UsageRange>('current')
  const theme = useTheme()
  const query = useQuery(listWalletDailyUsageCost, walletDailyUsageInput(wallet, scope, range))
  const response = query.data
  const data =
    response &&
    walletUsageChartData(
      response.rows,
      response.from && response.to ? { start: response.from, end: response.to } : undefined,
      MAX_SERIES
    )

  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const precision = wallet.creditUnit?.precision ?? currencyPrecision(wallet.currency)
  const palette = theme.isDarkMode ? DARK_SERIES : LIGHT_SERIES
  const colorOrder = data ? seriesColorOrder(data.series) : []
  const colorOf = (key: string) =>
    key === OTHER_SERIES
      ? OTHER_COLOR[theme.isDarkMode ? 'dark' : 'light']
      : palette[Math.max(0, colorOrder.indexOf(key)) % palette.length]
  const stackKeys = data?.series.some(s => s.key === OTHER_SERIES)
    ? [...colorOrder, OTHER_SERIES]
    : colorOrder
  const days = data?.days ?? []
  const rows = days.map(day => {
    const row: Record<string, number | string> = { date: day.date }
    for (const key of stackKeys) row[key] = Number(day.values[key] ?? 0n) / 10 ** precision
    return row
  })
  const valuesByDay = new Map(days.map(day => [day.date, day.values]))

  const tooltip = ({ data: bar }: BarTooltipProps<Record<string, number | string>>) => {
    const date = String(bar.date)
    const values = valuesByDay.get(date) ?? {}
    const entries = stackKeys
      .map(key => ({ key, value: values[key] ?? 0n }))
      .filter(entry => entry.value !== 0n)
      .reverse()
    const total = entries.reduce((sum, entry) => sum + entry.value, 0n)
    return (
      <div className="min-w-[160px] rounded border border-border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-sm">
        <div className="mb-1 font-medium">{formatShortDay(date)}</div>
        {entries.map(entry => (
          <div key={entry.key} className="flex items-center justify-between gap-3 py-px">
            <span className="flex items-center gap-1.5">
              <span
                className="inline-block h-2 w-2 shrink-0 rounded-full"
                style={{ backgroundColor: colorOf(entry.key) }}
              />
              {entry.key}
            </span>
            <span className="tabular-nums">{amount(entry.value)}</span>
          </div>
        ))}
        {entries.length > 1 && (
          <div className="mt-1 flex justify-between gap-3 border-t border-border pt-1 font-medium">
            <span>Total</span>
            <span className="tabular-nums">{amount(total)}</span>
          </div>
        )}
        {entries.length === 0 && <div className="text-muted-foreground">No usage</div>}
      </div>
    )
  }

  const loading = query.isLoading
  const error = query.isError
  const tickEvery = rows.length <= 10 ? 1 : rows.length <= 31 ? 7 : 14
  const visibleTicks = new Set(
    rows.map((row, i) => (i % tickEvery === 0 ? String(row.date) : undefined)).filter(Boolean)
  )
  const compactOptions: Intl.NumberFormatOptions = wallet.creditUnit
    ? { notation: 'compact', maximumFractionDigits: 1 }
    : {
        notation: 'compact',
        maximumFractionDigits: 1,
        style: 'currency',
        currency: wallet.currency,
      }
  const compact = new Intl.NumberFormat(undefined, compactOptions)
  const period = RANGE_LABEL[range].toLowerCase()

  return (
    <section className="space-y-3" aria-label="Credit usage">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-baseline gap-2">
          <h4 className="text-sm font-medium text-foreground">Credit usage</h4>
          {data && data.total > 0n && (
            <span className="text-xs text-muted-foreground tabular-nums">
              {amount(data.total)} {period}
              {wallet.creditUnit ? '' : ' · excl. tax'}
            </span>
          )}
        </div>
        <Select value={range} onValueChange={value => setRange(value as UsageRange)}>
          <SelectTrigger className="h-8 w-[140px] text-xs" aria-label="Period">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(Object.keys(RANGE_LABEL) as UsageRange[]).map(key => (
              <SelectItem key={key} value={key}>
                {RANGE_LABEL[key]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {loading ? (
        <Skeleton height={200} />
      ) : error ? (
        <p className="text-xs text-muted-foreground">Usage could not be loaded.</p>
      ) : !data || data.total === 0n ? (
        <p className="py-6 text-center text-xs text-muted-foreground">No usage {period}.</p>
      ) : (
        <div className="grid gap-4 lg:grid-cols-3">
          <div
            className="h-[200px] lg:col-span-2"
            role="img"
            aria-label={`Credit usage per day, ${period}: ${data.series
              .map(s => `${s.key} ${amount(s.total)}`)
              .join(', ')}`}
          >
            <ResponsiveBar
              data={rows}
              keys={stackKeys}
              indexBy="date"
              margin={{ top: 8, right: 8, bottom: 24, left: 52 }}
              padding={0.3}
              innerPadding={1}
              borderRadius={2}
              colors={bar => colorOf(String(bar.id))}
              axisBottom={{
                tickSize: 0,
                tickPadding: 8,
                format: v => (visibleTicks.has(String(v)) ? formatShortDay(String(v)) : ''),
              }}
              axisLeft={{
                tickSize: 0,
                tickPadding: 8,
                tickValues: 4,
                format: v => compact.format(Number(v)),
              }}
              gridYValues={4}
              enableLabel={false}
              tooltip={tooltip}
              theme={chartTheme}
            />
          </div>
          <TopConsumers series={data.series} colorOf={colorOf} amount={amount} />
        </div>
      )}
    </section>
  )
}

const TopConsumers = ({
  series,
  colorOf,
  amount,
}: {
  series: { key: string; total: bigint; share: number }[]
  colorOf: (key: string) => string
  amount: (minor: bigint) => string
}) => (
  <div className="space-y-2">
    <h5 className="text-xs text-muted-foreground">Top consumers</h5>
    <ul className="space-y-1.5 text-xs">
      {series.map(s => (
        <li key={s.key} className="flex items-center justify-between gap-3">
          <span className="flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden
              className="inline-block h-2 w-2 shrink-0 rounded-full"
              style={{ backgroundColor: colorOf(s.key) }}
            />
            <span className="truncate text-foreground" title={s.key}>
              {s.key}
            </span>
          </span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            <span className="text-foreground">{amount(s.total)}</span> · {Math.round(s.share)}%
          </span>
        </li>
      ))}
    </ul>
  </div>
)
