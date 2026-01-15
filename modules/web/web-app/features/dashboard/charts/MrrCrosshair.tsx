import { ComputedSerie, Datum } from '@nivo/line'
import dayjs from 'dayjs'
import { useMemo, useRef } from 'react'

import { useCurrency } from '@/hooks/useCurrency'

import { Crosshair } from './Crosshair'
import { ChartInterval, getTooltipFormat, keepWithinRange } from './utils'

interface MrrCrosshairProps {
  interval: ChartInterval
  containerRef: React.RefObject<HTMLDivElement>
  serie: ComputedSerie[]
  tooltip: TooltipProps
}

interface TooltipProps {
  format: 'currency' | 'percent' | 'number'
  labels: Record<string, string>
  render?: (data: Datum) => React.ReactNode
}

export const MrrCrosshair: React.FC<MrrCrosshairProps> = ({
  serie,
  interval,
  containerRef,
  tooltip,
}: MrrCrosshairProps) => {
  const { formatAmount } = useCurrency()

  const tooltipRef = useRef<HTMLDivElement>(null)
  const first = serie?.[0]

  const tooltipLeft = useMemo(() => {
    const windowWidth = window.innerWidth
    const containerBox = containerRef.current?.getBoundingClientRect()
    const tooltipRect = tooltipRef.current?.getBoundingClientRect()

    if (!tooltipRect) {
      return -999999
    }

    const offset = 8

    return keepWithinRange(first?.data[0].position.x + offset, {
      min: 0,
      max: windowWidth - (containerBox?.left ?? 0) - tooltipRect.width - offset,
    })
  }, [first, tooltipRef.current, containerRef.current])

  if (!serie || !serie[0]) {
    return null
  }

  return (
    <>
      <Crosshair.Line
        style={{
          left: `${serie[0].data[0].position.x}px`,
        }}
      />
      <Crosshair.Tooltip style={{ left: tooltipLeft }} ref={tooltipRef}>
        <h3 className="text-sm font-semibold mb-3">
          {dayjs(first.data[0].data.x).format(getTooltipFormat(interval))}
        </h3>
        {serie.map(serie => {
          if (serie?.data[0].data.y === null) return null
          return (
            <div key={serie.id}>
              <div className="flex justify-between">
                <span className="font-semibold">{tooltip.labels[serie.id] ?? 'unknown serie'}</span>
                <span>
                  {/*  TODO we should not need /100 */}
                  {tooltip.format === 'currency' &&
                    formatAmount((serie?.data[0].data.y as number) / 100)}{' '}
                  {tooltip.format === 'percent' && (serie?.data[0].data.y as number)}
                  {tooltip.format === 'number' && (serie?.data[0].data.y as number)}
                </span>
              </div>
              {tooltip.render && tooltip.render(serie.data[0].data)}
            </div>
          )
        })}
      </Crosshair.Tooltip>
    </>
  )
}
