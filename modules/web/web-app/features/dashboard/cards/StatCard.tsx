import { Skeleton, cn } from '@md/ui'
import { ArrowRight } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router'

import { DashboardCard, DashboardCardTitle } from '@/features/dashboard/cards/DashboardCard'
import { useCurrency } from '@/hooks/useCurrency'

export interface TrendProp {
  value: number
  percent: number
  period: string
  positiveIsGreen: boolean
}
export interface StatCardProp {
  title: string
  detailPath?: string
  value?: string | number
  secondaryValue?: string
  trend?: TrendProp
  loading?: boolean
}
export const StatCard: React.FC<StatCardProp> = ({
  title,
  detailPath,
  value,
  secondaryValue,
  trend,
  loading,
}) => {
  const card = (
    <DashboardCard
      raised
      className={cn(
        'flex h-[125px] grow flex-col justify-between',
        detailPath && 'transition-colors duration-200 group-hover:border-foreground/20'
      )}
    >
      <div className="flex w-full flex-row items-center justify-between">
        <DashboardCardTitle>{title}</DashboardCardTitle>
        {detailPath && (
          // Where the card leads: a quiet arrow that steps to the right on hover.
          <ArrowRight
            aria-hidden
            size={14}
            className="text-faint-foreground transition-[translate,color] duration-200 ease-out group-hover:translate-x-0.5 group-hover:text-foreground/70 motion-reduce:transition-none"
          />
        )}
      </div>
      <div>
        {loading ? (
          <div className="w-full flex items-end gap-4 pb-1">
            <Skeleton width="100%" height="1.5rem" />
            <Skeleton width="50%" height="1rem" />
          </div>
        ) : (
          <>
            <div className="flex flex-row gap-4 items-baseline">
              {value && <div className="font-display text-[21px] leading-none">{value}</div>}

              {secondaryValue && (
                <div className="text-xs text-muted-foreground self-baseline">{secondaryValue}</div>
              )}
            </div>
            {trend && <StatCardTrend {...trend} />}
          </>
        )}
      </div>
    </DashboardCard>
  )

  // With a destination the whole card is the link.
  return detailPath ? (
    <Link
      to={detailPath}
      className="group flex grow rounded-[6px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      {card}
    </Link>
  ) : (
    card
  )
}

export const StatCardTrend = ({ value, percent, period, positiveIsGreen }: TrendProp) => {
  const { formatAmount } = useCurrency()

  const formattedTrend = useMemo(() => {
    const formattedValue = formatAmount(Math.abs(value))

    const sign = value >= 0 ? '+' : '-'

    return `${sign} ${formattedValue} (${sign}${percent.toFixed(1)}%) ${period}`
  }, [value, percent, period, formatAmount])

  return (
    <div
      className={cn(
        'text-xs',
        value === 0
          ? 'text-muted-foreground'
          : positiveIsGreen === value > 0
            ? 'text-success dark:text-[#02d590]'
            : 'text-tag-destructive'
      )}
    >
      {formattedTrend}
    </div>
  )
}
