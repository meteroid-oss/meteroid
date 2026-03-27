import {
  BatchJobChunkStatus,
  BatchJobStatus,
  BatchJobType,
} from '@/rpc/api/batchjobs/v1/models_pb'

type StatusBadgeVariant =
  | 'secondary'
  | 'default'
  | 'success'
  | 'warning'
  | 'destructive'
  | 'outline'

export const BATCH_JOB_STATUS_CONFIG: Record<
  number,
  { label: string; variant: StatusBadgeVariant }
> = {
  [BatchJobStatus.PENDING]: { label: 'Pending', variant: 'secondary' },
  [BatchJobStatus.CHUNKING]: { label: 'Preparing', variant: 'default' },
  [BatchJobStatus.PROCESSING]: { label: 'Processing', variant: 'default' },
  [BatchJobStatus.COMPLETED]: { label: 'Completed', variant: 'success' },
  [BatchJobStatus.COMPLETED_WITH_ERRORS]: { label: 'Completed with Errors', variant: 'warning' },
  [BatchJobStatus.FAILED]: { label: 'Failed', variant: 'destructive' },
  [BatchJobStatus.CANCELLED]: { label: 'Cancelled', variant: 'secondary' },
}

export const ACTIVE_STATUSES: BatchJobStatus[] = [
  BatchJobStatus.PENDING,
  BatchJobStatus.CHUNKING,
  BatchJobStatus.PROCESSING,
]

export const JOB_TYPE_LABELS: Record<number, string> = {
  [BatchJobType.EVENT_CSV_IMPORT]: 'Event Import (CSV)',
  [BatchJobType.CUSTOMER_CSV_IMPORT]: 'Customer Import (CSV)',
  [BatchJobType.SUBSCRIPTION_CSV_IMPORT]: 'Subscription Import (CSV)',
  [BatchJobType.SUBSCRIPTION_PLAN_MIGRATION]: 'Subscription Plan Migration',
}

export const CHUNK_STATUS_CONFIG: Record<
  number,
  { label: string; variant: StatusBadgeVariant }
> = {
  [BatchJobChunkStatus.CHUNK_PENDING]: { label: 'Pending', variant: 'secondary' },
  [BatchJobChunkStatus.CHUNK_PROCESSING]: { label: 'Processing', variant: 'default' },
  [BatchJobChunkStatus.CHUNK_COMPLETED]: { label: 'Completed', variant: 'success' },
  [BatchJobChunkStatus.CHUNK_FAILED]: { label: 'Failed', variant: 'destructive' },
  [BatchJobChunkStatus.CHUNK_SKIPPED]: { label: 'Skipped', variant: 'secondary' },
}

export const CHUNK_EVENT_CONFIG: Record<string, { label: string; variant: StatusBadgeVariant }> = {
  STARTED: { label: 'Started', variant: 'default' },
  COMPLETED: { label: 'Completed', variant: 'success' },
  ERRORED: { label: 'Error', variant: 'destructive' },
  RETRYING: { label: 'Retrying', variant: 'warning' },
  EXHAUSTED: { label: 'Exhausted', variant: 'destructive' },
  SKIPPED: { label: 'Skipped', variant: 'secondary' },
}

export function getStatusConfig(status: BatchJobStatus) {
  return (
    BATCH_JOB_STATUS_CONFIG[status] ?? {
      label: String(status),
      variant: 'secondary' as StatusBadgeVariant,
    }
  )
}

export function getChunkStatusConfig(status: BatchJobChunkStatus) {
  return (
    CHUNK_STATUS_CONFIG[status] ?? {
      label: String(status),
      variant: 'secondary' as StatusBadgeVariant,
    }
  )
}

export { BatchJobStatus, BatchJobType, BatchJobChunkStatus }
