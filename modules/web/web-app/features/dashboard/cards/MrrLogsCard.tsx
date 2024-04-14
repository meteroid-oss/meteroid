import { format } from 'date-fns'

import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'
import { useQuery } from '@/lib/connectrpc'
import { mapDateFromGrpc } from '@/lib/mapping'
import { mrrLog } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { MRRMovementType } from '@/rpc/api/stats/v1/models_pb'
import { Badge, ScrollArea } from '@ui/components'
import { cn } from '@ui/lib'
import { Link } from 'react-router-dom'

const mrrTypeTolabel: Record<MRRMovementType, string> = {
  [MRRMovementType.NEW_BUSINESS]: 'New business',
  [MRRMovementType.EXPANSION]: 'Expansion',
  [MRRMovementType.CONTRACTION]: 'Contraction',
  [MRRMovementType.CHURN]: 'Churn',
  [MRRMovementType.REACTIVATION]: 'Reactivation',
}
const mrrTypeToColor: Record<MRRMovementType, string> = {
  [MRRMovementType.NEW_BUSINESS]: 'bg-green-700',
  [MRRMovementType.EXPANSION]: 'bg-blue-700',
  [MRRMovementType.CONTRACTION]: 'bg-purple-700',
  [MRRMovementType.CHURN]: 'bg-red-700',
  [MRRMovementType.REACTIVATION]: 'bg-yellow-700',
}

const Circle = ({ movementType }: { movementType: MRRMovementType }) => (
  <div
    className={cn(
      'w-[10px] h-[10px] rounded-full shadow-circle mr-2 opacity-60',
      mrrTypeToColor[movementType]
    )}
  ></div>
)

export const MrrLogsCard = () => {
  const logs = useQuery(mrrLog, {}).data

  return (
    <div className="max-w-[50%] relative h-[180px] w-[50%]  py-4 px-2 ">
      <div className="text-sm font-semibold leading-none tracking-tight">MRR Movement Logs</div>
      <div className="pt-5 h-full">
        <div className="h-full ">
          <ScrollArea className="h-full pr-2 -mr-4">
            {logs?.entries?.length ? (
              logs.entries.map((log, idx) => (
                <div
                  key={idx}
                  className="text-xs flex flex-row gap-2 p-1  items-baseline box-border rounded-sm justify-between hover:bg-muted"
                >
                  <span className="flex items-center w-[90px]">
                    <Circle movementType={log.mrrType} />
                    {log.appliesTo && format(mapDateFromGrpc(log.appliesTo), 'dd/MM/yyyy')}
                  </span>
                  <span className="flex  w-[90px]">{mrrTypeTolabel[log.mrrType]}</span>
                  <span className="flex flex-grow gap-1">
                    {log.description} for customer
                    <Link
                      to={`customers/${log.customerId}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="font-semibold underline decoration-border decoration-dashed underline-offset-2"
                    >
                      {log.customerName}
                    </Link>
                  </span>
                </div>
              ))
            ) : (
              <div className="h-full items-center justify-end">
                <ChartNoData />
              </div>
            )}
          </ScrollArea>
        </div>
      </div>
    </div>
  )
}
