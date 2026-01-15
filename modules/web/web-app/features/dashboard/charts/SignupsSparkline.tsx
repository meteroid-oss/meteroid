import { linearGradientDef } from '@nivo/core'
import { ComputedSerie, ResponsiveLine } from '@nivo/line'
import { useRef, useState } from 'react'

import { MrrCrosshair } from '@/features/dashboard/charts/MrrCrosshair'
import { ActiveSerieLayer } from '@/features/dashboard/charts/utils'
import { useQuery } from '@/lib/connectrpc'
import { signupSparkline } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

export const SignupsSparkline = () => {
  const theme = useTheme()

  const chartData = useQuery(signupSparkline)
  const [serie, setSerie] = useState<ComputedSerie[]>([])
  const containerRef = useRef<HTMLDivElement>(null)

  const series = chartData.data?.series
    ? [
        {
          id: chartData.data.series.code,
          data: chartData.data.series.data.map(d => ({
            x: d.x,
            y: Number(d.delta ?? 0),
            key: d.x,
          })),
        },
      ]
    : []

  const isEmpty = !chartData.data?.series?.data?.length

  if (isEmpty) {
    return (
      <div className="w-full  align-center bottom-0 font-semibold text-sm text-center  ">
        no data
      </div>
    )
  }

  return (
    <div className="h-full relative" ref={containerRef}>
      <MrrCrosshair
        serie={serie}
        interval="All"
        containerRef={containerRef}
        tooltip={{
          format: 'number',
          labels: {
            [chartData.data?.series?.code ?? 'new_signups']: 'New signups',
          },
        }}
      />
      <ResponsiveLine
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
        colors={[theme.isDarkMode ? '#8b8a74' : '#513ceb']}
        lineWidth={1}
        data={series}
        layers={['areas', 'lines', props => <ActiveSerieLayer {...props} setSerie={setSerie} />]}
      />
    </div>
  )
}
