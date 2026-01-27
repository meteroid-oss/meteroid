import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@md/ui'
import { linearGradientDef } from '@nivo/core'
import { ComputedSerie, LineSvgProps, ResponsiveLine } from '@nivo/line'
import { ChevronDownIcon } from '@radix-ui/react-icons'
import { useMemo, useRef, useState } from 'react'

import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { MrrCrosshair } from '@/features/dashboard/charts/MrrCrosshair'
import { ChartType } from '@/features/dashboard/charts/types'
import { ActiveSerieLayer } from '@/features/dashboard/charts/utils'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { mapDate } from '@/lib/mapping'
import { totalRevenueChart } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

interface RevenueChartProps {
  from: Date
  to: Date
  plansId: string[]
  chartType?: ChartType
  onChartTypeChange?: (type: ChartType) => void
}

const commonChartProps: LineSvgProps = {
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

export const RevenueChart = ({
  from,
  to,
  plansId,
  chartType,
  onChartTypeChange,
}: RevenueChartProps) => {
  const theme = useTheme()

  const chartData = useQuery(totalRevenueChart, {
    startDate: mapDate(from),
    endDate: mapDate(to),
    plansId: plansId,
  })
  const { formatAmount } = useCurrency()

  const data =
    chartData.data?.series.map(s => ({
      id: s.code,
      data: s.data.map(d => ({
        x: d.x,
        y: Number(d.revenue ?? 0),
        key: d.x,
        dailyRevenue: Number(d.dailyRevenue ?? 0),
      })),
    })) ?? []

  const { min, max }: { min: number; max: number } = useMemo(() => {
    const numbers = data
      ?.map(d => d.data)
      .flat()
      ?.filter(d => d?.y !== null)
      .map(point => Number(point.y))

    if (!numbers || numbers.length === 0) {
      return { min: 0, max: 0 }
    }

    const maxVal = Math.max(...numbers)
    const minVal = Math.min(...numbers)

    // Add some padding to the range
    const padding = (maxVal - minVal) * 0.1
    return {
      min: Math.max(0, minVal - padding),
      max: maxVal + padding,
    }
  }, [data])

  const [serie, setSerie] = useState<ComputedSerie[]>([])

  const isEmpty = !chartData.data?.series || chartData.data.series.every(s => s.data.length === 0)

  const containerRef = useRef<HTMLDivElement>(null)

  // Get the last data point's total revenue for display
  const lastDataPoint = data[0]?.data[data[0]?.data.length - 1]
  const firstDataPoint = data[0]?.data[0]
  const currentTotal = lastDataPoint?.y ?? 0
  const startTotal = firstDataPoint?.y ?? 0

  // Calculate change percentage based on the chart data
  const changePercent = useMemo(() => {
    if (!startTotal || startTotal === 0) return 0
    return ((currentTotal - startTotal) / startTotal) * 100
  }, [currentTotal, startTotal])

  const isPositive = changePercent >= 0

  return (
    <div>
      <div className="py-4 flex flex-row gap-12">
        <div className="flex flex-col gap-2">
          {onChartTypeChange ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button className="flex items-center gap-1 text-sm font-semibold text-muted-foreground hover:text-foreground transition-colors">
                  {chartType === 'revenue' ? 'Revenue' : 'MRR'}
                  <ChevronDownIcon className="h-3 w-3" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                <DropdownMenuItem onClick={() => onChartTypeChange('revenue')}>
                  Revenue
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => onChartTypeChange('mrr')}>MRR</DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : (
            <div className="text-sm font-semibold text-muted-foreground">Revenue</div>
          )}
          <div>
            <span className="text-2xl font-semibold leading-6">
              {formatAmount(currentTotal / 100)}
            </span>
          </div>
          <div className="text-sm text-muted-foreground">
            <span className={isPositive ? 'text-success' : 'text-destructive'}>
              {isPositive && '+'}
              {changePercent.toFixed(1)}%
            </span>
            <span className="ml-1">vs last period</span>
          </div>
        </div>
      </div>
      <div className="h-[220px] relative" ref={containerRef}>
        <div className="h-0 w-0">{!isEmpty && <div className="chart-dotted-bg" />}</div>
        <MrrCrosshair
          serie={serie}
          interval="All"
          containerRef={containerRef}
          tooltip={{
            format: 'currency',
            labels: {
              total_revenue: 'Cumulated ',
            },
            render: datum => {
              const dailyRevenue = (datum as { dailyRevenue?: number }).dailyRevenue
              if (dailyRevenue === undefined) return null
              return (
                <div className="flex justify-between text-muted-foreground text-xs mt-1">
                  <span>Daily</span>
                  <span>{formatAmount(dailyRevenue / 100)}</span>
                </div>
              )
            },
          }}
        />
        {isEmpty ? (
          <ChartNoData error={!!chartData.error} />
        ) : (
          <ResponsiveLine
            {...commonChartProps}
            theme={{
              axis: {
                ticks: {
                  text: {
                    fill: 'hsl(var(--muted-foreground))',
                  },
                },
              },
            }}
            areaOpacity={0.15}
            enableArea={true}
            defs={[
              linearGradientDef('gradientA', [
                { offset: 0, color: 'inherit' },
                { offset: 100, color: 'inherit', opacity: 0 },
              ]),
            ]}
            fill={[{ match: '*', id: 'gradientA' }]}
            colors={[theme.isDarkMode ? '#a78bfa' : '#7c3aed']}
            data={data}
            margin={{ top: 10, right: 50, bottom: 30, left: 50 }}
            xScale={{
              type: 'time',
              format: '%Y-%m-%d',
              precision: 'day',
              useUTC: false,
              min: data[0]?.data[0]?.x ? new Date(data[0].data[0].x) : 'auto',
              max: data[0]?.data[data[0]?.data.length - 1]?.x
                ? new Date(data[0].data[data[0].data.length - 1].x)
                : 'auto',
            }}
            xFormat="time:%b %d, %Y"
            yScale={{ type: 'linear', min: min, max: max }}
            axisBottom={{
              tickSize: 0,
              tickPadding: 10,
              tickRotation: 0,
              format: '%b %d, %Y',
              tickValues: [
                data[0]?.data[0]?.x ? new Date(data[0].data[0].x) : null,
                data[0]?.data[data[0]?.data.length - 1]?.x
                  ? new Date(data[0].data[data[0].data.length - 1].x)
                  : null,
              ].filter(Boolean) as Date[],
            }}
            axisLeft={null}
            layers={[
              'areas',
              'lines',
              'axes',
              props => <ActiveSerieLayer {...props} setSerie={setSerie} />,
            ]}
          />
        )}
      </div>
    </div>
  )
}
