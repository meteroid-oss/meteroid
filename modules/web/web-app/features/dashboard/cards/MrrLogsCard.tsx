import { ScrollArea, Tooltip, TooltipContent, TooltipTrigger } from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import { ArrowDownIcon, ArrowUpIcon, MinusIcon, PlusIcon, RefreshCwIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { mapDateFromGrpc } from '@/lib/mapping'
import { MRRMovementType } from '@/rpc/api/stats/v1/models_pb'
import { mrrLog } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

const mrrTypeConfig: Record<
  MRRMovementType,
  { label: string; color: string; bgColor: string; icon: React.ElementType }
> = {
  [MRRMovementType.NEW_BUSINESS]: {
    label: 'New',
    color: 'text-green-600',
    bgColor: 'bg-green-500/10',
    icon: PlusIcon,
  },
  [MRRMovementType.EXPANSION]: {
    label: 'Expansion',
    color: 'text-blue-600',
    bgColor: 'bg-blue-500/10',
    icon: ArrowUpIcon,
  },
  [MRRMovementType.CONTRACTION]: {
    label: 'Contraction',
    color: 'text-orange-600',
    bgColor: 'bg-orange-500/10',
    icon: ArrowDownIcon,
  },
  [MRRMovementType.CHURN]: {
    label: 'Churn',
    color: 'text-red-600',
    bgColor: 'bg-red-500/10',
    icon: MinusIcon,
  },
  [MRRMovementType.REACTIVATION]: {
    label: 'Reactivation',
    color: 'text-emerald-600',
    bgColor: 'bg-emerald-500/10',
    icon: RefreshCwIcon,
  },
}

const MovementBadge = ({ type }: { type: MRRMovementType }) => {
  const config = mrrTypeConfig[type]
  const Icon = config.icon

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className={cn(
            'flex items-center justify-center w-6 h-6 rounded-md shrink-0',
            config.bgColor
          )}
        >
          <Icon className={cn('w-3.5 h-3.5', config.color)} />
        </div>
      </TooltipTrigger>
      <TooltipContent side="top" className="text-xs">
        {config.label}
      </TooltipContent>
    </Tooltip>
  )
}

export const MrrLogsCard = () => {
  const logs = useQuery(mrrLog, {}).data
  const { formatAmount } = useCurrency()

  const formatMrrChange = (amount: bigint, type: MRRMovementType) => {
    const isNegative = type === MRRMovementType.CHURN || type === MRRMovementType.CONTRACTION
    const formatted = formatAmount(amount < 0n ? -amount : amount)
    return isNegative ? `-${formatted}` : `+${formatted}`
  }

  return (
    <div className="max-w-[50%] relative h-[180px] w-[50%] pt-4 px-2">
      <div className="text-sm font-semibold leading-none tracking-tight mb-3">
        MRR Movement Logs
      </div>
      <ScrollArea className="h-[calc(100%-28px)] pr-2 -mr-4">
        {logs?.entries?.length ? (
          <div className="space-y-1.5">
            {logs.entries.map((log, idx) => {
              const config = mrrTypeConfig[log.mrrType]
              return (
                <div
                  key={idx}
                  className="flex items-center gap-2.5 p-2 rounded-lg hover:bg-muted/50 transition-colors"
                >
                  <MovementBadge type={log.mrrType} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5 text-xs">
                      <Link
                        to={`customers/${log.customerId}`}
                        className="font-medium truncate hover:underline underline-offset-2"
                      >
                        {log.customerName}
                      </Link>
                      <span className="text-muted-foreground shrink-0">on</span>
                      <span className="text-muted-foreground truncate">{log.planName}</span>
                    </div>
                    <div className="text-[10px] text-muted-foreground mt-0.5">
                      {log.appliesTo && format(mapDateFromGrpc(log.appliesTo), 'MMM d, yyyy')}
                    </div>
                  </div>
                  <div className={cn('text-xs font-semibold tabular-nums shrink-0', config.color)}>
                    {formatMrrChange(log.mrrChange, log.mrrType)}
                  </div>
                </div>
              )
            })}
          </div>
        ) : (
          <div className="h-full flex items-center justify-center">
            <ChartNoData />
          </div>
        )}
      </ScrollArea>
    </div>
  )
}
