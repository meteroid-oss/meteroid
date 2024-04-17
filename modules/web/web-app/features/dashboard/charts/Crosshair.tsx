import ChartCrosshair, { tabular } from '@/features/dashboard/charts/ChartCrosshair'
import { formatCurrency } from '@/features/dashboard/utils'
import dayjs from 'dayjs'
import styled from 'styled-components'
import {
  ComputedDatum,
  ComputedSerie,
  CustomLayerProps,
  Datum,
  Line,
  LineSvgProps,
  Serie,
} from '@nivo/line'
import { useMemo, useRef } from 'react'

export enum ChartInterval {
  '1D' = '1D',
  '1W' = '1W',
  '1M' = '1M',
  '3M' = '3M',
  '6M' = '6M',
  '1Y' = '1Y',
  '2Y' = '2Y',
  '3Y' = '3Y',
  '5Y' = '5Y',
  '10Y' = '10Y',
  WTD = 'WTD',
  YTD = 'YTD',
  All = 'All',
  Custom = 'Custom',
}

interface CrosshairProps {
  serie: ComputedSerie[]
  interval: ChartInterval
  containerRef: React.RefObject<HTMLDivElement>
  tooltip: ChartTooptip
}

interface ChartTooptip {
  format: 'currency' | 'percent' | 'time'
  labels: Record<string, string>
  render?: (data: Datum) => React.ReactNode
}

export const clamp = (value: number, min: number, max: number): number => {
  return value < min ? min : value > max ? max : value
}

export const CHART_FORMAT_INTERVAL = {
  [ChartInterval['1D']]: {
    label: {
      split: 'hour',
      format: 'hh',
    },
    tooltip: { format: 'h:mm A' },
  },
  [ChartInterval['1W']]: {
    label: {
      split: 'day',
      format: 'ddd',
    },
    tooltip: { format: 'MMM D, h:mm A' },
  },
  [ChartInterval['WTD']]: {
    label: {
      split: 'day',
      format: 'ddd',
    },
    tooltip: { format: 'MMM D, h:mm A' },
  },
  [ChartInterval['1M']]: {
    label: {
      split: 'day',
      format: 'D',
    },
    tooltip: { format: 'MMM D, h:mm A' },
  },
  [ChartInterval['3M']]: {
    label: {
      split: 'month',
      format: 'MMM',
    },
    tooltip: { format: 'MMM D, h:mm A' },
  },
  [ChartInterval['6M']]: {
    label: {
      split: 'month',
      format: 'MMM',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval['1Y']]: {
    label: {
      split: 'month',
      format: 'MMM',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval['2Y']]: {
    label: {
      split: 'month',
      format: 'MMM',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval['3Y']]: {
    label: {
      split: 'month',
      format: 'MMM',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval['5Y']]: {
    label: {
      split: 'year',
      format: 'YYYY',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval['10Y']]: {
    label: {
      split: 'year',
      format: 'YYYY',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval.YTD]: {
    label: {
      split: 'year',
      format: 'YYYY',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval.All]: {
    label: {
      split: 'year',
      format: 'YYYY',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
  [ChartInterval.Custom]: {
    label: {
      split: 'year',
      format: 'YYYY',
    },
    tooltip: { format: 'MMM D, YYYY' },
  },
}

export const Crosshair: React.FC<CrosshairProps> = ({
  serie,
  interval,
  containerRef,
  tooltip,
}: CrosshairProps) => {
  const tooltipSettings = CHART_FORMAT_INTERVAL[interval]?.tooltip
  const tooltipRef = useRef<HTMLDivElement>(null)
  const first = serie?.[0]

  const tooltipLeft = useMemo(() => {
    const tooltipRect = tooltipRef.current?.getBoundingClientRect()
    const windowWidth = window.innerWidth
    const containerBox = containerRef.current?.getBoundingClientRect()

    if (!tooltipRect) {
      return -999999
    }

    return clamp(
      first?.data[0].position.x + 8,
      0,
      windowWidth - containerBox?.left! - tooltipRect?.width - 8
    )
  }, [first, tooltipRef.current, containerRef.current])

  if (!serie || !serie[0]) {
    return null
  }

  return (
    <>
      <ChartCrosshair.Line
        style={{
          left: `${serie[0].data[0].position.x}px`,
          background: 'hsl(var(--muted-foreground) / var(--tw-text-opacity))',
        }}
      />
      <CrosshairTooltip style={{ left: tooltipLeft }} ref={tooltipRef}>
        <h3 className="text-sm font-semibold mb-3">
          {dayjs(first.data[0].data.x).format(tooltipSettings.format)}
        </h3>
        {serie.map(serie => {
          const labelkey = Object.keys(tooltip.labels).find(l => (serie.id as string).includes(l))
          const label = tooltip.labels[labelkey!]

          if (serie?.data[0].data.y === null) {
            return null
          }

          return (
            <>
              <TooltipItem key={serie.id}>
                {/* <Text size={14} color="white" weight={600}>
                  {label}
                </Text> */}
                <span className="font-semibold">{label}</span>
                <span>
                  {tooltip.format === 'currency' &&
                    // <Currency
                    //   value={serie?.data[0].data.y as number}
                    //   animate={false}
                    //   color={ema ? theme.colors.states.live : serie.color}
                    //   hidePlus
                    //   as="strong"
                    // />
                    formatCurrency(serie?.data[0].data.y as number)}
                  {tooltip.format === 'percent' &&
                    // <Percent
                    //   value={serie?.data[0].data.y as number}
                    //   animate={false}
                    //   color={ema ? theme.colors.states.live : serie.color}
                    //   as="strong"
                    // />
                    (serie?.data[0].data.y as number)}
                </span>
              </TooltipItem>

              {tooltip.render && tooltip.render(serie.data[0].data)}
            </>
          )
        })}
      </CrosshairTooltip>
    </>
  )
}

const CrosshairTooltip = styled.div`
  z-index: 1;
  position: absolute;
  top: 0;
  left: 8px;
  font-size: 14px;
  background: hsl(var(--popover) / 0.8);
  backdrop-filter: blur(1.5px);
  box-shadow:
    0px 4px 25px rgba(0, 0, 0, 0.25),
    inset 0 0 1px rgba(255, 255, 255, 0.04);
  position: absolute;
  padding: 16px 18px 10px 18px;
  min-width: 204px;
  border-radius: 7px;
  pointer-events: none;

  span,
  strong {
    ${tabular}
  }
`

const TooltipItem = styled.div`
  display: flex;
  justify-content: space-between;

  &:not(:last-of-type) {
    margin-bottom: 4px;
  }
`
