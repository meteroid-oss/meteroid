import { linearGradientDef } from '@nivo/core'
import { ComputedSeries, LineSeries, ResponsiveLine } from '@nivo/line'
import { useMemo, useRef, useState } from 'react'

import { ChartEmptyState } from '@/components/charts/ChartEmptyState'
import { MrrCrosshair } from '@/features/dashboard/charts/MrrCrosshair'
import { ActiveSerieLayer } from '@/features/dashboard/charts/utils'
import { useQuery } from '@/lib/connectrpc'
import { trialConversionRateSparkline } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

export const TrialConversionSparkline = () => {
  const theme = useTheme()

  const chartData = useQuery(trialConversionRateSparkline)
  const [serie, setSerie] = useState<ComputedSeries<LineSeries>[]>([])
  const containerRef = useRef<HTMLDivElement>(null)

  const series =
    chartData.data?.series.map(s => ({
      id: s.code,
      data: s.data.map(d => ({
        x: d.x,
        y: Number(d.conversionRate ?? 0),
        key: d.x,
      })),
    })) ?? []

  // Build labels map from series data
  const seriesLabels = useMemo(() => {
    const labels: Record<string, string> = {}
    chartData.data?.series.forEach(s => {
      labels[s.code] = s.name
    })
    return labels
  }, [chartData.data?.series])

  const isEmpty =
    !chartData.data?.series ||
    chartData.data.series.every(
      s => s.data.length === 0 || (s.data.length === 1 && s.data[0].conversions === BigInt(0))
    )

  if (isEmpty) {
    return <ChartEmptyState compact error={!!chartData.error} title="No trial conversions yet" />
  }

  return (
    <div className="h-full relative" ref={containerRef}>
      <MrrCrosshair
        serie={serie}
        interval="All"
        containerRef={containerRef}
        tooltip={{
          format: 'percent',
          labels: seriesLabels,
        }}
      />
      <ResponsiveLine<LineSeries>
        enableGridX={false}
        enableCrosshair={false}
        enablePoints={false}
        enableGridY={false}
        enableArea={true}
        areaOpacity={0.3}
        defs={[
          linearGradientDef('gradientZ', [
            { offset: 0, color: 'inherit' },
            { offset: 100, color: 'inherit', opacity: 0 },
          ]),
        ]}
        fill={[{ match: '*', id: 'gradientZ' }]}
        colors={[theme.isDarkMode ? '#a78bfa' : '#7c3aed']}
        lineWidth={1}
        data={series}
        layers={['areas', 'lines', props => <ActiveSerieLayer {...props} setSerie={setSerie} />]}
      />
    </div>
  )
}
