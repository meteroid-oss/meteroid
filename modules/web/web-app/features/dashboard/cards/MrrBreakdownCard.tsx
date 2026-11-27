import { cn, Separator } from '@md/ui'

import { DashboardCard, DashboardCardTitle } from '@/features/dashboard/cards/DashboardCard'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { mapDate } from '@/lib/mapping'
import { mrrBreakdown } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

interface MrrBreakdownCardProps {
  from: Date
  to: Date
}

export const MrrBreakdownCard = ({ from, to }: MrrBreakdownCardProps) => {
  const breakdown = useQuery(mrrBreakdown, {
    startDate: mapDate(from),
    endDate: mapDate(to),
  }).data?.mmrBreakdown

  const { formatAmount } = useCurrency()

  return (
    <DashboardCard className="h-[290px]">
      <DashboardCardTitle>MRR Breakdown</DashboardCardTitle>
      <div className="pt-5">
        <div className="h-[90px]">
          <MrrBreakdownItem
            title="New business"
            count={breakdown?.newBusiness?.count ?? 0}
            valueCents={formatAmount(breakdown?.newBusiness?.value)}
            type="new"
          />
          <Separator />
          <MrrBreakdownItem
            title="Expansions"
            count={breakdown?.expansion?.count ?? 0}
            valueCents={formatAmount(breakdown?.expansion?.value)}
            type="expansion"
          />
          <Separator />
          <MrrBreakdownItem
            title="Reactivations"
            count={breakdown?.reactivation?.count ?? 0}
            valueCents={formatAmount(breakdown?.reactivation?.value)}
            type="reactivation"
          />
          <Separator />
          <MrrBreakdownItem
            title="Contractions"
            count={breakdown?.contraction?.count ?? 0}
            valueCents={formatAmount(breakdown?.contraction?.value)}
            type="contraction"
          />
          <Separator />
          <MrrBreakdownItem
            title="Churn"
            count={breakdown?.churn?.count ?? 0}
            valueCents={formatAmount(breakdown?.churn?.value)}
            type="churn"
          />
        </div>
      </div>
    </DashboardCard>
  )
}

interface MrrBreakdownItemProp {
  title: string
  count: number | bigint
  valueCents: string
  type: keyof typeof colors
}
const MrrBreakdownItem = ({ title, count, valueCents, type }: MrrBreakdownItemProp) => {
  return (
    <div className="p-1 flex flex-row items-baseline box-border text-xs rounded-sm justify-between hover:bg-muted ">
      <div className="flex flex-row items-center space-x-2 ">
        <Circle colorClassName={colors[type]} />
        <span>{title}</span>
        <span className="font-medium">({Number(count)})</span>
      </div>
      <div>{valueCents}</div>
    </div>
  )
}

// MRR movements in the tag palette: new green, expansion teal, reactivation neutral,
// contraction orange, churn red (also used by the MRR chart's tooltip).
const colors = {
  new: 'bg-tag-success',
  expansion: 'bg-tag-info',
  reactivation: 'bg-tag-neutral',
  churn: 'bg-tag-destructive',
  contraction: 'bg-tag-warning',
}

// same in tailwind :
const Circle = ({ colorClassName }: { colorClassName: string }) => (
  <div className={cn('w-2 h-2 rounded-full mr-1', colorClassName)}></div>
)

export const MrrColorCircle = ({ type }: { type: keyof typeof colors }) => (
  <Circle colorClassName={colors[type]} />
)
export type MrrColorCircleColors = keyof typeof colors
