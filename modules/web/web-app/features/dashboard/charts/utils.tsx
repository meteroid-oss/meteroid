import { ComputedDatum, ComputedSerie, CustomLayerProps } from '@nivo/line'
import dayjs from 'dayjs'
import { useRef } from 'react'

const DEFAULT_PADDING = 8

export type ChartInterval =
  | '1D'
  | '1W'
  | '1M'
  | '3M'
  | '6M'
  | '1Y'
  | '2Y'
  | '3Y'
  | 'WTD'
  | 'YTD'
  | 'All'
  | 'Custom'

export const getTooltipFormat = (interval: ChartInterval) => {
  switch (interval) {
    case '1D':
      return 'MMM D, h:mm A'
    default:
      return 'MMM D, YYYY'
  }
}

export const keepWithinRange = (value: number, range: { min: number; max: number }) => {
  return Math.min(Math.max(value, range.min), range.max)
}

export const formatDate = (date: Date, interval: ChartInterval): string => {
  return dayjs(date).format(getTooltipFormat(interval))
}

interface ActiveSerieLayerProps extends CustomLayerProps {
  setSerie: React.Dispatch<React.SetStateAction<ComputedSerie[]>>
}

/**
 * Layer that finds the closest serie to the pointer and sets it as the active serie
 */
export const ActiveSerieLayer: React.FC<ActiveSerieLayerProps> = ({
  innerHeight,
  innerWidth,
  series,
  setSerie,
}: ActiveSerieLayerProps) => {
  const rect = useRef<SVGRectElement>(null)

  const width = innerWidth + DEFAULT_PADDING * 2
  const getSerie = (
    event: React.PointerEvent<SVGRectElement> | React.MouseEvent
  ): ComputedSerie[] => {
    const layerBounds = rect.current!.getBoundingClientRect()
    const xOffset = event.clientX - layerBounds.x - DEFAULT_PADDING

    return series.map(serie => {
      const nearestDatum = serie.data.reduce((prev, curr) => {
        const offsetCurr = curr.position.x - xOffset
        const offsetPrev = prev.position.x - xOffset
        return Math.abs(offsetCurr) < Math.abs(offsetPrev) ? curr : prev
      }, serie.data?.[0] as ComputedDatum)

      return {
        ...serie,
        data: [nearestDatum],
      }
    })
  }

  return (
    <g transform={`translate(-${DEFAULT_PADDING},0)`}>
      <rect
        ref={rect}
        height={innerHeight}
        width={width}
        onPointerMove={ev => setSerie(getSerie(ev))}
        onPointerUp={ev => rect.current!.releasePointerCapture(ev.pointerId)}
        onPointerDown={ev => rect.current!.setPointerCapture(ev.pointerId)}
        onPointerLeave={() => setSerie([])}
        fillOpacity={0}
      />
    </g>
  )
}
