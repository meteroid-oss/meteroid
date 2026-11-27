import { cn } from '@md/ui'

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
    <div className={cn('relative h-[180px] grow flex flex-col p-5', className)}>
      <div className="text-[13px] leading-none tracking-[-0.13px] text-foreground/60">{title}</div>
      <div className="min-h-[20px] flex flex-row pr-6 py-4 items-baseline w-full justify-between grow ">
        <div className="font-display text-[21px] leading-none">{value}</div>
        {secondaryValue}
      </div>
      <div>
        <div className="h-[90px]">{chart}</div>
      </div>
    </div>
  )
}
