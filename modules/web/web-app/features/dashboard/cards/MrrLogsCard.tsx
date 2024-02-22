import { TopCard } from '@/features/dashboard/cards/TopCard'
import { formatCurrency } from '@/features/dashboard/utils'
import { useQuery } from '@/lib/connectrpc'
import { mapDateFromGrpc } from '@/lib/mapping'
import { MRRBreakdownScope } from '@/rpc/api/stats/v1/models_pb'
import { mrrLog } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { cn } from '@ui/lib'
import { format } from 'date-fns'

export const MrrLogsCard = () => {
  const logs = useQuery(mrrLog, {}).data

  return (
    <div className="max-w-[50%] relative h-[180px] w-[450px] min-w-[250px] container border-b border-l border-slate-500  flex flex-col py-4 px-6">
      <div className="text-sm font-semibold leading-none tracking-tight">MRR Movement Logs</div>
      <div className="pt-5">
        <div className="h-[90px]">
          {logs?.entries?.length ? (
            logs.entries.map(log => (
              <div className="text-xs">
                {log.appliesTo && format(mapDateFromGrpc(log.appliesTo), 'dd/MM/yyyy')}:{' '}
                {log.mrrType} - {log.description}
              </div>
            ))
          ) : (
            <div className="text-sm flex font-semibold h-full items-center justify-center">
              No data
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
