import { cn } from '@md/ui'

import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { MRRBreakdownScope } from '@/rpc/api/stats/v1/models_pb'
import { mrrBreakdown } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { Separator } from '@md/ui'

export const MrrBreakdownCard = () => {
  const breakdown = useQuery(mrrBreakdown, { scope: MRRBreakdownScope.THIS_MONTH }).data
    ?.mmrBreakdown

  return (
    <div className="max-w-[50%] relative h-[180px] w-[50%] py-4 px-2 ">
      <div className="text-sm font-semibold leading-none tracking-tight">MRR Breakdown</div>
      <div className="pt-5">
        <div className="h-[90px]">
          <MrrBreakdownItem
            title="New business"
            count={breakdown?.newBusiness?.count ?? 0}
            valueCents={formatCurrency(breakdown?.newBusiness?.value)}
            type="new"
          />
          <Separator />
          <MrrBreakdownItem
            title="Expansions"
            count={breakdown?.expansion?.count ?? 0}
            valueCents={formatCurrency(breakdown?.expansion?.value)}
            type="expansion"
          />
          <Separator />
          <MrrBreakdownItem
            title="Reactivations"
            count={breakdown?.reactivation?.count ?? 0}
            valueCents={formatCurrency(breakdown?.reactivation?.value)}
            type="reactivation"
          />
          <Separator />
          <MrrBreakdownItem
            title="Contractions"
            count={breakdown?.contraction?.count ?? 0}
            valueCents={formatCurrency(breakdown?.contraction?.value)}
            type="contraction"
          />
          <Separator />
          <MrrBreakdownItem
            title="Churn"
            count={breakdown?.churn?.count ?? 0}
            valueCents={formatCurrency(breakdown?.churn?.value)}
            type="churn"
          />
        </div>
      </div>
    </div>
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

const colors = {
  new: 'bg-green-700',
  expansion: 'bg-blue-700',
  reactivation: 'bg-yellow-700',
  churn: 'bg-red-700',
  contraction: 'bg-purple-700',
}

// same in tailwind :
const Circle = ({ colorClassName }: { colorClassName: string }) => (
  <div
    className={cn('w-[12px] h-[12px] rounded-full shadow-circle mr-2 opacity-60', colorClassName)}
  ></div>
)
