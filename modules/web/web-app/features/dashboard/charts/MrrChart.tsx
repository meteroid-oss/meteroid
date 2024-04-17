import { colors } from '@md/foundation'
import { linearGradientDef } from '@nivo/core'
import {
  ComputedDatum,
  ComputedSerie,
  CustomLayerProps,
  LineSvgProps,
  ResponsiveLine,
} from '@nivo/line'
import { styled } from '@stitches/react'

import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { mapDate } from '@/lib/mapping'
import { generalStats, totalMrrChart } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { useTheme } from 'providers/ThemeProvider'
import { useMemo, useRef, useState } from 'react'
import { ChartInterval, Crosshair } from '@/features/dashboard/charts/Crosshair'
import { MRRBreakdown } from '@/rpc/api/stats/v1/models_pb'
import { MrrColorCircle, MrrColorCircleColors } from '@/features/dashboard/cards/MrrBreakdownCard'

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

const commonChartProperties: LineSvgProps = {
  data: [],
  lineWidth: 1,
  animate: false,
  enableGridX: false,
  enableGridY: false,
  axisLeft: null,
  axisBottom: null,
  enableSlices: false,
  enableCrosshair: false,
  enablePoints: false,
  //theme: chartTheme,
  colors: { datum: 'color' },
}
export const MrrChart = (props: MrrChartProps) => {
  const theme = useTheme()

  const stats = useQuery(generalStats)
  const chartData = useQuery(totalMrrChart, {
    plansId: props.plansId,
    startDate: mapDate(props.from),
    endDate: mapDate(props.to),
  })

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
    console.log('data', data)
    return (
      <div className="flex flex-col gap-2 text-muted-foreground text-xs border-t border-border pt-3">
        <Item label="Net New MRR" value={formatCurrency(data.breakdown.netNewMrr)} />

        {!!data.breakdown.newBusiness?.count && (
          <Item
            circle="new"
            label="New Business"
            value={formatCurrency(data.breakdown.newBusiness.value)}
            count={data.breakdown.newBusiness.count}
          />
        )}
        {!!data.breakdown.expansion?.count && (
          <Item
            circle="expansion"
            label="Expansions"
            value={formatCurrency(data.breakdown.expansion.value)}
            count={data.breakdown.expansion.count}
          />
        )}
        {!!data.breakdown.contraction?.count && (
          <Item
            circle="contraction"
            label="Contractions"
            value={formatCurrency(data.breakdown.contraction.value)}
            count={data.breakdown.contraction.count}
          />
        )}
        {!!data.breakdown.churn?.count && (
          <Item
            circle="churn"
            label="Churn"
            value={formatCurrency(data.breakdown.churn.value)}
            count={data.breakdown.churn.count}
          />
        )}
        {!!data.breakdown.reactivation?.count && (
          <Item
            circle="reactivation"
            label="Reactivations"
            value={formatCurrency(data.breakdown.reactivation.value)}
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

    if (!numbers) {
      return { min: 0, max: 0 }
    }

    let max = Math.max(...numbers)
    let min = Math.min(...numbers)

    return {
      min,
      max,
    }
  }, [data])

  const [serie, setSerie] = useState<ComputedSerie[]>([])

  const isEmpty = !chartData.data?.series || chartData.data.series.every(s => s.data.length === 0)

  const containerRef = useRef<HTMLDivElement>(null)

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
      <div className="h-[220px] relative" ref={containerRef}>
        <div className="h-0 w-0">{!isEmpty && <DottedBackground />}</div>
        <Crosshair
          serie={serie}
          interval={ChartInterval.All}
          containerRef={containerRef}
          tooltip={{
            format: 'currency',
            labels: {
              total_mrr: 'Total MRR',
            },
            render: d => renderTooltipAdditionalData(d as any),
          }}
        />
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
            data={data}
            xScale={{
              type: 'time', // Set the x-axis scale to 'time'
              format: '%Y-%m-%d', // Specify the date format
              precision: 'day', // Set the precision to 'day'
              nice: true, // Enable the 'nice' feature
            }}
            yScale={{ type: 'linear', min: min, max: max }}
            layers={[
              'lines',
              props => (
                <FindNearestSeriesToPointer {...props} setSerie={setSerie} /> //chartWidth={width}
              ),
            ]}
          />
        )}
      </div>
    </div>
  )
}

interface FindNearestPointProps extends CustomLayerProps {
  //chartWidth: number
  setSerie: React.Dispatch<React.SetStateAction<ComputedSerie[]>>
}

export const FindNearestSeriesToPointer: React.FC<FindNearestPointProps> = ({
  series,
  setSerie,
  innerHeight,
  //chartWidth,
  innerWidth,
}: FindNearestPointProps) => {
  const layerRef = useRef<SVGRectElement>(null)
  const padding = 16

  // This is responsible for showing the Crosshair at the right point on the X axis
  function findNearestSeriesToPointer(
    event: React.PointerEvent<SVGRectElement> | React.MouseEvent
  ): ComputedSerie[] {
    const layerBounds = layerRef.current!.getBoundingClientRect()
    const xOffset = event.clientX - layerBounds.x - padding / 2

    return series.map(serie => {
      const data = serie.data.reduce((prev, curr) => {
        return Math.abs((curr.position.x as number) - xOffset) <
          Math.abs((prev.position.x as number) - xOffset)
          ? curr
          : prev
      }, serie.data?.[0] as ComputedDatum)

      serie.data = [data]
      return serie
    })
  }

  // on mouse down we need to set the comparison point
  function handlePointerDown(event: React.PointerEvent<SVGRectElement>) {
    layerRef.current!.setPointerCapture(event.pointerId)
  }

  // on mouse down we need to set the comparison point
  function handlePointerUp(event: React.PointerEvent<SVGRectElement>) {
    layerRef.current!.releasePointerCapture(event.pointerId)
  }

  function handlePointerMove(event: React.PointerEvent<SVGRectElement>) {
    return setSerie(findNearestSeriesToPointer(event))
  }

  function handlePointerLeave() {
    setSerie(null as any)
  }

  return (
    <g transform={`translate(-${padding / 2},0)`}>
      <rect
        ref={layerRef}
        onPointerDown={handlePointerDown}
        onPointerUp={handlePointerUp}
        onPointerMove={handlePointerMove}
        onPointerLeave={handlePointerLeave}
        width={innerWidth + padding} // smaller buffer to make it easier to see latest point
        height={innerHeight}
        fillOpacity={0}
      />
    </g>
  )
}
