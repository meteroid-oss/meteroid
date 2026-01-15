import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@md/ui'
import { linearGradientDef } from '@nivo/core'
import { ComputedSerie, LineSvgProps, ResponsiveLine } from '@nivo/line'
import { ChevronDownIcon } from '@radix-ui/react-icons'
import { useMemo, useRef, useState } from 'react'

import { MrrColorCircle, MrrColorCircleColors } from '@/features/dashboard/cards/MrrBreakdownCard'
import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { MrrCrosshair } from '@/features/dashboard/charts/MrrCrosshair'
import { ChartType } from '@/features/dashboard/charts/types'
import { ActiveSerieLayer } from '@/features/dashboard/charts/utils'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { mapDate } from '@/lib/mapping'
import { MRRBreakdown } from '@/rpc/api/stats/v1/models_pb'
import { totalMrrChart } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

interface MrrChartProps {
  plansId: string[]
  from: Date
  to: Date
  chartType?: ChartType
  onChartTypeChange?: (type: ChartType) => void
  refetchInterval?: number | false
}

const commonChartProps: LineSvgProps = {
  data: [],
  lineWidth: 1,
  animate: false,
  axisLeft: null,
  axisBottom: null,
  enableCrosshair: false,
  enableGridX: false,
  enableGridY: false,
  enableSlices: false,
  enablePoints: false,
  colors: { datum: 'color' },
}

export const MrrChart = ({
  plansId,
  from,
  to,
  chartType,
  onChartTypeChange,
  refetchInterval,
}: MrrChartProps) => {
  const theme = useTheme()

  const chartData = useQuery(
    totalMrrChart,
    {
      plansId: plansId,
      startDate: mapDate(from),
      endDate: mapDate(to),
    },
    {
      refetchInterval,
    }
  )
  const { formatAmount } = useCurrency()

  const data =
    chartData.data?.series.map(s => ({
      id: s.code,
      data: s.data.map(d => ({
        x: d.x,
        y: Number(d.data?.totalNetMrr ?? 0),
        key: d.x,
        breakdown: d.data,
      })),
    })) ?? []

  // Build labels map from series data (code -> name)
  const seriesLabels = useMemo(() => {
    const labels: Record<string, string> = {}
    chartData.data?.series.forEach(s => {
      labels[s.code] = s.name
    })
    return labels
  }, [chartData.data?.series])

  const Item = ({
    label,
    value,
    circle,
    count,
  }: {
    label: string
    value: string
    count?: bigint
    circle?: MrrColorCircleColors
  }) => (
    <div className="flex justify-between items-center space-x-2" key={label}>
      <span className="flex justify-between items-center space-x-0">
        {circle && <MrrColorCircle type={circle} />}
        <span className="semibold pr-2">{label}</span>
      </span>
      <span>{value}</span>
      {count ? <span className="font-medium">({Number(count)})</span> : null}
    </div>
  )

  const renderTooltipAdditionalData = (data: { breakdown: MRRBreakdown }) => {
    return (
      <div className="flex flex-col gap-2 text-muted-foreground text-xs border-t border-border pt-3">
        <Item label="Net New MRR" value={formatAmount(data.breakdown.netNewMrr)} />

        {!!data.breakdown.newBusiness?.count && (
          <Item
            circle="new"
            label="New Business"
            value={formatAmount(data.breakdown.newBusiness.value)}
            count={data.breakdown.newBusiness.count}
          />
        )}
        {!!data.breakdown.expansion?.count && (
          <Item
            circle="expansion"
            label="Expansions"
            value={formatAmount(data.breakdown.expansion.value)}
            count={data.breakdown.expansion.count}
          />
        )}
        {!!data.breakdown.contraction?.count && (
          <Item
            circle="contraction"
            label="Contractions"
            value={formatAmount(data.breakdown.contraction.value)}
            count={data.breakdown.contraction.count}
          />
        )}
        {!!data.breakdown.churn?.count && (
          <Item
            circle="churn"
            label="Churn"
            value={formatAmount(data.breakdown.churn.value)}
            count={data.breakdown.churn.count}
          />
        )}
        {!!data.breakdown.reactivation?.count && (
          <Item
            circle="reactivation"
            label="Reactivations"
            value={formatAmount(data.breakdown.reactivation.value)}
            count={data.breakdown.reactivation.count}
          />
        )}
      </div>
    )
  }

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

    return {
      min: minVal,
      max: maxVal,
    }
  }, [data])

  const [serie, setSerie] = useState<ComputedSerie[]>([])

  const isEmpty = !chartData.data?.series || chartData.data.series.every(s => s.data.length === 0)

  const containerRef = useRef<HTMLDivElement>(null)

  // Get the last data point's total MRR for display
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
            <div className="text-sm font-semibold text-muted-foreground">MRR</div>
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
            labels: seriesLabels,
            render: d =>
              renderTooltipAdditionalData(
                d as {
                  breakdown: MRRBreakdown
                }
              ),
          }}
        />
        {isEmpty ? (
          <ChartNoData error={!!chartData.error} />
        ) : (
          <ResponsiveLine
            {...commonChartProps}
            lineWidth={2}
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
            margin={{ top: 10, right: 10, bottom: 30, left: 10 }}
            xScale={{
              type: 'time',
              format: '%Y-%m-%d',
              precision: 'day',
              nice: true,
            }}
            xFormat="time:%b %d, %Y"
            yScale={{ type: 'linear', min: min, max: max }}
            axisBottom={{
              tickSize: 0,
              tickPadding: 10,
              tickRotation: 0,
              format: '%b %Y',
              tickValues: 'every month',
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
