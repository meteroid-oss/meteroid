import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { MRRBreakdownScope } from '@/rpc/api/stats/v1/models_pb'
import { mrrBreakdown } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { cn } from '@ui/lib'

export const MrrBreakdownCard = () => {
  const breakdown = useQuery(mrrBreakdown, { scope: MRRBreakdownScope.THIS_MONTH }).data
    ?.mmrBreakdown

  return (
    <div className="max-w-[50%] relative h-[180px] w-[450px] min-w-[250px] container border-b border-slate-500 flex flex-col py-4 px-6">
      <div className="text-sm font-semibold leading-none tracking-tight">MRR Breakdown</div>
      <div className="pt-5">
        <div className="h-[90px]">
          <MrrBreakdownItem
            title="New business"
            count={breakdown?.newBusiness?.count ?? 0}
            valueCents={formatCurrency(breakdown?.newBusiness?.value)}
            type="new"
          />
          <MrrBreakdownItem
            title="Expansions"
            count={breakdown?.expansion?.count ?? 0}
            valueCents={formatCurrency(breakdown?.expansion?.value)}
            type="expansion"
          />
          <MrrBreakdownItem
            title="Reactivations"
            count={breakdown?.reactivation?.count ?? 0}
            valueCents={formatCurrency(breakdown?.reactivation?.value)}
            type="reactivation"
          />
          <MrrBreakdownItem
            title="Contractions"
            count={breakdown?.contraction?.count ?? 0}
            valueCents={formatCurrency(breakdown?.contraction?.value)}
            type="contraction"
          />
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
    <div className="p-0.5 flex flex-row gap-4 items-baseline box-border text-xs rounded-sm justify-between hover:bg-slate-200 ">
      <div className="flex flex-row items-center space-x-3 ">
        <Circle colorClassName={colors[type]} />
        <span className="font-semibold">
          {title} ({Number(count)})
        </span>
      </div>
      <div className="text-xs">{valueCents}</div>
    </div>
  )
}

const colors = {
  new: 'bg-green-1000',
  expansion: 'bg-blue-1000',
  reactivation: 'bg-yellow-1000',
  churn: 'bg-red-1000',
  contraction: 'bg-purple-1000',
}

// same in tailwind :
const Circle = ({ colorClassName }: { colorClassName: string }) => (
  <div
    className={cn('w-[12px] h-[12px] rounded-full shadow-circle mr-2 opacity-60', colorClassName)}
  ></div>
)
