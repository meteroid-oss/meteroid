import { ConnectError, Code } from '@connectrpc/connect'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import {
  createBatchJob,
  listBatchJobs,
} from '@/rpc/api/batchjobs/v1/batchjobs-BatchJobsService_connectquery'
import { CreateBatchJobRequest } from '@/rpc/api/batchjobs/v1/batchjobs_pb'

export function useBatchJobCreate(opts?: {
  onSuccess?: (jobId: string) => void
  onDuplicate?: () => void
}) {
  const queryClient = useQueryClient()

  return useMutation(createBatchJob, {
    onSuccess: async res => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listBatchJobs) })
      const jobId = res.job?.id
      if (jobId) {
        toast.success('Batch job created')
        opts?.onSuccess?.(jobId)
      }
    },
    onError: error => {
      if (error instanceof ConnectError && error.code === Code.AlreadyExists) {
        opts?.onDuplicate?.()
        return
      }
      toast.error(`Failed to create batch job: ${error.message}`)
    },
  })
}

export { CreateBatchJobRequest }
