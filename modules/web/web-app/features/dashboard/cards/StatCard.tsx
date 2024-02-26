import { ButtonAlt, Skeleton } from '@ui/components'
import { cn } from '@ui/lib'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

export interface TrendProp {
  value: number
  percent: number
  period: string
  positiveIsGreen: boolean
}
export interface StatCardProp {
  title: string
  detailPath?: string
  value: string | number
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
  return (
    <div className="h-[120px] w-[450px] min-w-[250px] rounded-lg border border-slate-400 flex flex-col">
      <div className="text-sm font-semibold flex flex-row px-6 py-4 items-baseline w-full justify-between flex-grow">
        <div className=" font-medium leading-none tracking-tight">{title}</div>
        {detailPath && (
          <Link to={detailPath}>
            <ButtonAlt type="text">
              <span className="underline decoration-slate-700 decoration-dashed underline-offset-2">
                View
              </span>
            </ButtonAlt>
          </Link>
        )}
      </div>
      <div className="px-6 pb-4">
        {loading ? (
          <div className="w-full flex items-end gap-4 pb-1">
            <Skeleton containerClassName="w-full" width="100%" height="1.5rem" />
            <Skeleton containerClassName="flex w-full items-end" width="50%" height="1rem" />
          </div>
        ) : (
          <>
            <div className="flex flex-row gap-4 items-baseline">
              <div className="text-2xl">{value}</div>
              {secondaryValue && (
                <div className="text-xs text-slate-1000 self-baseline">{secondaryValue}</div>
              )}
            </div>
            {trend && <StatCardTrend {...trend} />}
          </>
        )}
      </div>
    </div>
  )
}

export const StatCardTrend = ({ value, percent, period, positiveIsGreen }: TrendProp) => {
  const formattedTrend = useMemo(() => {
    const formattedValue = new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits: 2,
    }).format(Math.abs(value))

    return `${value >= 0 ? '+ ' : '- '}${formattedValue} (${percent}%) ${period}`
  }, [value, percent, period])

  return (
    <div
      className={cn(
        'text-xs',
        value === 0
          ? 'text-scale-1100'
          : positiveIsGreen === value > 0
            ? 'text-green-900'
            : 'text-red-500'
      )}
    >
      {formattedTrend}
    </div>
  )
}
