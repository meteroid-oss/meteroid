import { colors } from '@md/foundation'
import { linearGradientDef } from '@nivo/core'
import { ResponsiveLine } from '@nivo/line'
import { styled } from '@stitches/react'
import { ArrowUp } from 'lucide-react'

import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { mapDate } from '@/lib/mapping'
import { generalStats, totalMrrChart } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'
import { Badge } from '@md/ui'
import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'

const DottedBackground = styled('div', {
  maskImage: 'radial-gradient(rgb(0, 0, 0), transparent 62%)',
  position: 'absolute',
  width: '100%',
  height: '100%',
  top: '0px',
  left: '0px',
  padding: '70px',
  opacity: 0.8,
  background: `radial-gradient(${colors.neutral4} 1px, transparent 0px)10px 0px / 8px 8px transparent`,
})

interface MrrChartProps {
  plansId: string[]
  from: Date
  to: Date
}
export const MrrChart = (props: MrrChartProps) => {
  const theme = useTheme()

  const stats = useQuery(generalStats)
  const chartData = useQuery(totalMrrChart, {
    plansId: props.plansId,
    startDate: mapDate(props.from),
    endDate: mapDate(props.to),
  })

  const series =
    chartData.data?.series.map(s => ({
      id: s.code,
      data: s.data.map(d => ({
        x: d.x,
        y: Number(d.data?.netNewMrr ?? 0),
        key: d.x,
      })),
    })) ?? []

  const isEmpty = !chartData.data?.series || chartData.data.series.every(s => s.data.length === 0)

  return (
    <div>
      <div className="py-4 flex flex-row gap-12">
        <div className="flex flex-col gap-2">
          <div className="px-2 text-sm font-bold">MRR</div>
          <div className="px-2">
            <span className="text-2xl font-medium leading-6">
              {formatCurrency(stats.data?.totalMrr?.valueCents)}
            </span>
            <span className="text-success text-sm font-semibold leading-4 ml-2">+0%</span>
          </div>
        </div>
        <div className="flex flex-col gap-2">
          <div className="text-sm">Today</div>
          <div className="flex text-md gap-2">
            <span>€0.00</span>
          </div>
        </div>
      </div>
      <div className="h-[220px] relative">
        <div className="h-0 w-0">{!isEmpty && <DottedBackground />}</div>
        {isEmpty ? (
          <ChartNoData error={!!chartData.error} />
        ) : (
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
              linearGradientDef('gradientA', [
                { offset: 0, color: 'inherit' },
                { offset: 100, color: 'inherit', opacity: 0 },
              ]),
            ]}
            fill={[{ match: '*', id: 'gradientA' }]}
            colors={[theme.isDarkMode ? '#8b8a74' : '#513ceb']}
            lineWidth={1}
            data={series}
          />
        )}
      </div>
    </div>
  )
}
