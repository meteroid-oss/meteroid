import { disableQuery } from '@connectrpc/connect-query'

import { ACTIVE_STATUSES } from '@/features/batch-jobs/statusConfig'
import { useQuery } from '@/lib/connectrpc'
import { getBatchJob } from '@/rpc/api/batchjobs/v1/batchjobs-BatchJobsService_connectquery'

export function useBatchJobPoll(jobId: string | undefined) {
  const query = useQuery(getBatchJob, jobId ? { jobId } : disableQuery, {
    refetchInterval: data => {
      const status = data?.state?.data?.job?.status
      if (status !== undefined && ACTIVE_STATUSES.includes(status)) {
        return 3000
      }
      return false
    },
  })

  const isActive = query.data?.job?.status !== undefined
    ? ACTIVE_STATUSES.includes(query.data.job.status)
    : false

  return {
    ...query,
    isActive,
  }
}
