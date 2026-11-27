import { ScrollArea, Tooltip, TooltipContent, TooltipTrigger } from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import {
  AlertCircleIcon,
  ArrowDownIcon,
  ArrowUpDownIcon,
  ArrowUpIcon,
  MinusIcon,
  PlusIcon,
  RefreshCwIcon,
} from 'lucide-react'
import { Link } from 'react-router'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { DashboardCard, DashboardCardTitle } from '@/features/dashboard/cards/DashboardCard'
import { useBasePath } from '@/hooks/useBasePath'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { mapDateFromGrpc } from '@/lib/mapping'
import { formatCurrency } from '@/lib/utils/numbers'
import { MRRMovementType } from '@/rpc/api/stats/v1/models_pb'
import { mrrLog } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'

const mrrTypeConfig: Record<
  MRRMovementType,
  { label: string; color: string; bgColor: string; icon: React.ElementType }
> = {
  [MRRMovementType.NEW_BUSINESS]: {
    label: 'New',
    color: 'text-tag-success',
    bgColor: 'bg-tag-success-bg',
    icon: PlusIcon,
  },
  [MRRMovementType.EXPANSION]: {
    label: 'Expansion',
    color: 'text-tag-info',
    bgColor: 'bg-tag-info-bg/70',
    icon: ArrowUpIcon,
  },
  [MRRMovementType.CONTRACTION]: {
    label: 'Contraction',
    color: 'text-tag-warning',
    bgColor: 'bg-tag-warning-bg',
    icon: ArrowDownIcon,
  },
  [MRRMovementType.CHURN]: {
    label: 'Churn',
    color: 'text-tag-destructive',
    bgColor: 'bg-tag-destructive-bg',
    icon: MinusIcon,
  },
  [MRRMovementType.REACTIVATION]: {
    label: 'Reactivation',
    color: 'text-tag-neutral',
    bgColor: 'bg-tag-neutral-bg',
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
  const logsQuery = useQuery(mrrLog, {})
  const logs = logsQuery.data
  const { currency: reportingCurrency } = useCurrency()

  const basePath = useBasePath()

  // Each movement is in its subscription's currency, which may differ from the tenant's.
  const formatMrrChange = (amount: bigint, type: MRRMovementType, currency: string) => {
    const code = currency || reportingCurrency
    if (!code) return '...'
    const isNegative = type === MRRMovementType.CHURN || type === MRRMovementType.CONTRACTION
    const formatted = formatCurrency(amount < 0n ? -amount : amount, code)
    return isNegative ? `-${formatted}` : `+${formatted}`
  }

  return (
    <DashboardCard className="flex h-[290px] flex-col gap-3">
      <DashboardCardTitle>MRR Movement Logs</DashboardCardTitle>
      <ScrollArea className="min-h-0 flex-1 pr-2 -mr-4">
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
                        to={`${basePath}/customers/${log.customerId}`}
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
                    {formatMrrChange(log.mrrChange, log.mrrType, log.currency)}
                  </div>
                </div>
              )
            })}
          </div>
        ) : (
          // A list, not a chart: the app's list empty state rather than the ghost chart.
          <div className="h-full flex items-center justify-center">
            {logsQuery.error ? (
              <PanelEmptyState
                icon={AlertCircleIcon}
                tone="destructive"
                title="Couldn't load MRR movements"
                description="Something went wrong fetching the data."
                className="py-8"
              />
            ) : (
              <PanelEmptyState
                icon={ArrowUpDownIcon}
                title="No MRR movements yet"
                description="New subscriptions, upgrades, downgrades and churn show up here."
                className="py-8"
              />
            )}
          </div>
        )}
      </ScrollArea>
    </DashboardCard>
  )
}
