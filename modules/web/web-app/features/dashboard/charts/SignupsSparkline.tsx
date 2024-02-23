import { linearGradientDef } from '@nivo/core'
import { ResponsiveLine } from '@nivo/line'

import { useQuery } from '@/lib/connectrpc'
import { signupSparkline } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'

export const SignupsSparkline = () => {
  const theme = useTheme()

  const chartData = useQuery(signupSparkline)

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

  console.log('series', series)

  const isEmpty = !chartData.data?.series?.data?.length

  if (isEmpty) {
    return (
      <div className="w-full  align-center bottom-0 font-semibold text-sm text-center  ">
        no data
      </div>
    )
  }

  return (
    <>
      <ResponsiveLine
        enableGridX={false}
        enableCrosshair={false}
        enablePoints={false}
        enableGridY={false}
        enableArea={true}
        useMesh
        areaOpacity={0.3}
        //   curve="monotoneX"
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
      />
    </>
  )
}
