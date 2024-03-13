import { format } from 'date-fns'

import { useQuery } from '@/lib/connectrpc'
import { mapDateFromGrpc } from '@/lib/mapping'
import { mrrLog } from '@/rpc/api/stats/v1/stats-StatsService_connectquery'
import { ChartNoData } from '@/features/dashboard/charts/ChartNoData'

export const MrrLogsCard = () => {
  const logs = useQuery(mrrLog, {}).data

  return (
    <div className="max-w-[50%] relative h-[180px] w-[50%]  py-4 px-2 ">
      <div className="text-sm font-semibold leading-none tracking-tight">MRR Movement Logs</div>
      <div className="pt-5 h-full">
        <div className="h-full container overflow-y-auto">
          {logs?.entries?.length ? (
            logs.entries.map((log, idx) => (
              <div key={idx} className="text-xs">
                {log.appliesTo && format(mapDateFromGrpc(log.appliesTo), 'dd/MM/yyyy')}:{' '}
                {log.mrrType} - {log.description}
              </div>
            ))
          ) : (
            <div className="h-full items-center justify-end">
              <ChartNoData />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
