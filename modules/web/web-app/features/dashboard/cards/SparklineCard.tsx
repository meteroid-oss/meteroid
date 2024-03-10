import { cn } from '@ui/lib'

interface SparklineCardProp {
  title: string
  detailPath?: string
  value: string
  secondaryValue?: React.ReactNode
  className?: string
  chart: React.ReactNode
}

export const SparklineCard: React.FC<SparklineCardProp> = ({
  title,
  value,
  secondaryValue,
  className,
  chart,
}) => {
  return (
    <div
      className={cn(
        'relative h-[180px] w-[450px] min-w-[250px] flex flex-col py-4 px-6',
        className
      )}
    >
      <div className="text-sm font-semibold">{title}</div>
      <div className="min-h-[60px] flex flex-row pr-6 py-4 items-baseline w-full justify-between flex-grow ">
        <div className="text-md font-medium">{value}</div>
        {secondaryValue}
      </div>
      <div>
        <div className="h-[90px]">{chart}</div>
      </div>
    </div>
  )
}
