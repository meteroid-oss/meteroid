import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Progress,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { PaginationState } from '@tanstack/react-table'
import { DownloadIcon, ExternalLinkIcon, FileIcon, RefreshCwIcon, XCircleIcon } from 'lucide-react'
import { FunctionComponent, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { BatchProgressBar } from '@/features/batch-jobs/BatchProgressBar'
import {
  ACTIVE_STATUSES,
  BatchJobChunkStatus,
  BatchJobStatus,
  getStatusConfig,
} from '@/features/batch-jobs/statusConfig'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import {
  cancelBatchJob,
  getBatchJob,
  listBatchJobFailures,
  retryBatchJob,
} from '@/rpc/api/batchjobs/v1/batchjobs-BatchJobsService_connectquery'

interface BatchJobProgressProps {
  jobId: string
}

export const BatchJobProgress: FunctionComponent<BatchJobProgressProps> = ({ jobId }) => {
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const [failurePagination, setFailurePagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 5,
  })

  const jobQuery = useQuery(
    getBatchJob,
    { jobId },
    {
      refetchInterval: data => {
        const status = data?.state?.data?.job?.status
        if (status !== undefined && ACTIVE_STATUSES.includes(status)) {
          return 3000
        }
        return false
      },
    }
  )

  const job = jobQuery.data?.job
  const chunks = jobQuery.data?.chunks ?? []
  const jobStatus = job?.status
  const isJobActive = jobStatus !== undefined && ACTIVE_STATUSES.includes(jobStatus)
  const hasFailures = (job?.failedItems ?? 0) > 0

  const failuresQuery = useQuery(
    listBatchJobFailures,
    {
      jobId,
      limit: failurePagination.pageSize,
      offset: failurePagination.pageIndex * failurePagination.pageSize,
    },
    {
      enabled: hasFailures,
      refetchInterval: isJobActive ? 5000 : false,
    }
  )

  const hasRetried = useRef(false)

  const cancelMut = useMutation(cancelBatchJob, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getBatchJob) })
      toast.success('Job cancelled')
    },
    onError: error => toast.error(`Failed to cancel: ${error.message}`),
  })

  const retryMut = useMutation(retryBatchJob, {
    onSuccess: async () => {
      hasRetried.current = true
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getBatchJob) }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(listBatchJobFailures),
        }),
      ])
      toast.success('Retrying failed items')
    },
    onError: error => toast.error(`Failed to retry: ${error.message}`),
  })

  if (jobQuery.isLoading) {
    return <Skeleton height={120} />
  }

  if (!job) {
    return (
      <div className="border border-border rounded-lg p-6 text-center">
        <p className="text-sm text-muted-foreground">Job not found</p>
      </div>
    )
  }

  const { label: statusLabel, variant: statusVariant } = getStatusConfig(job.status)
  const isActive = ACTIVE_STATUSES.includes(job.status)
  const canRetry = !hasRetried.current && job.status === BatchJobStatus.FAILED

  const total = job.totalItems ?? 0
  const processed = job.processedItems + job.failedItems
  const progressPct = total > 0 ? Math.round((processed / total) * 100) : 0

  const failures = failuresQuery.data?.failures ?? []
  const failureTotalCount = Number(failuresQuery.data?.totalCount ?? 0)

  const errorCsvUrl = job.errorCsvToken
    ? `${env.meteroidRestApiUri}/files/v1/batch-job/errors/${jobId}?token=${job.errorCsvToken}`
    : null

  const inputFileUrl = job.inputFileToken
    ? `${env.meteroidRestApiUri}/files/v1/batch-job/input/${jobId}?token=${job.inputFileToken}`
    : null

  return (
    <div className="space-y-4">
      <div className="space-y-3">
        {inputFileUrl && (
          <div className="flex items-center gap-1.5 text-xs">
            <FileIcon size={12} className="text-muted-foreground shrink-0" />
            <a
              href={inputFileUrl}
              download={job.inputFileName ?? 'input.csv'}
              className="text-muted-foreground hover:text-foreground underline underline-offset-2"
            >
              {job.inputFileName ?? 'Source file'}
            </a>
          </div>
        )}
        <div className="flex items-center justify-between">
          <Badge variant={statusVariant}>{statusLabel}</Badge>
          <div className="flex items-center gap-2">
            <Link
              to={`${basePath}/developers/batch-jobs/${jobId}`}
              className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1"
            >
              View job details <ExternalLinkIcon size={12} />
            </Link>
            {canRetry && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => retryMut.mutate({ jobId })}
                disabled={retryMut.isPending}
              >
                <RefreshCwIcon size={14} className="mr-1" />
                Retry (without change)
              </Button>
            )}
            {isActive && (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => cancelMut.mutate({ jobId })}
                disabled={cancelMut.isPending}
              >
                Cancel
              </Button>
            )}
          </div>
        </div>

        <div className="space-y-1">
          {total > 0 ? (
            <>
              <div className="flex justify-between text-xs text-muted-foreground">
                <span>
                  {job.processedItems.toLocaleString()} processed
                  {job.failedItems > 0 && (
                    <span className="text-destructive">
                      , {job.failedItems.toLocaleString()} failed
                    </span>
                  )}
                  {' / '}
                  {total.toLocaleString()} total
                </span>
                <span>{progressPct}%</span>
              </div>
              <BatchProgressBar
                processed={job.processedItems}
                failed={job.failedItems}
                total={total}
              />
            </>
          ) : (
            isActive && (
              <>
                <p className="text-xs text-muted-foreground">Preparing import…</p>
                <Progress indeterminate />
              </>
            )
          )}
        </div>

        {/* Active chunks indicator */}
        {isActive &&
          chunks.length > 0 &&
          (() => {
            const processing = chunks.filter(c => c.status === BatchJobChunkStatus.CHUNK_PROCESSING)
            const pending = chunks.filter(c => c.status === BatchJobChunkStatus.CHUNK_PENDING)
            const completed = chunks.filter(
              c =>
                c.status === BatchJobChunkStatus.CHUNK_COMPLETED ||
                c.status === BatchJobChunkStatus.CHUNK_FAILED
            )
            if (processing.length === 0 && pending.length === 0) return null
            return (
              <div className="text-xs text-muted-foreground">
                {completed.length}/{chunks.length} batches done
                {processing.length > 0 && (
                  <span>
                    {' · '}
                    {processing.length} processing
                    {processing.some(c => c.retryCount > 0) && (
                      <span className="text-warning"> (retrying)</span>
                    )}
                  </span>
                )}
                {pending.length > 0 && (
                  <span>
                    {' · '}
                    {pending.length} pending
                  </span>
                )}
              </div>
            )
          })()}

        {job.errorMessage && (
          <div className="flex items-start gap-2 p-2 border border-destructive/50 rounded-md bg-destructive/5">
            <XCircleIcon size={14} className="text-destructive mt-0.5 shrink-0" />
            <p className="text-xs text-destructive">{job.errorMessage}</p>
          </div>
        )}

        {/* Chunk-level error summary (e.g. retries exhausted, service down) */}
        {(() => {
          const errorMessages = chunks
            .flatMap(c => c.events)
            .filter(e => e.event === 'ERRORED' || e.event === 'EXHAUSTED')
            .map(e => e.message)
            .filter((m): m is string => !!m)
          const unique = [...new Set(errorMessages)]
          if (unique.length === 0) return null
          return (
            <div className="space-y-1">
              {unique.map((msg, i) => (
                <div
                  key={i}
                  className="flex items-start gap-2 p-2 border border-destructive/50 rounded-md bg-destructive/5"
                >
                  <XCircleIcon size={14} className="text-destructive mt-0.5 shrink-0" />
                  <p className="text-xs text-destructive">{msg}</p>
                </div>
              ))}
            </div>
          )
        })()}

        {/* Retry progress indicator */}
        {isActive &&
          chunks.some(c => c.retryCount > 0) &&
          (() => {
            const retrying = chunks.filter(c => c.retryAfter)
            if (retrying.length === 0) return null
            return (
              <p className="text-xs text-warning">
                {retrying.length} batch(es) retrying (attempt{' '}
                {Math.max(...retrying.map(c => c.retryCount)) + 1})…
              </p>
            )
          })()}
      </div>

      {/* Fix & re-import section */}
      {!isActive && errorCsvUrl && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium">Fix & re-import</h4>
          <a href={errorCsvUrl} download={`errors-${jobId}.csv`}>
            <Button variant="outline" size="sm">
              <DownloadIcon size={14} className="mr-1" />
              Download failed rows
            </Button>
          </a>
          <p className="text-xs text-muted-foreground">
            CSV containing only the failed rows with an _error column. Fix the errors and re-import.
          </p>
        </div>
      )}

      {failureTotalCount > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">Failures ({failureTotalCount})</h3>
          <Table containerClassName="border border-border rounded-lg">
            <TableHeader>
              <TableRow>
                <TableHead className="w-[80px] text-xs">Row</TableHead>
                <TableHead className="w-[180px] text-xs">Identifier</TableHead>
                <TableHead className="text-xs">Error</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {failures.map(failure => (
                <TableRow key={failure.id}>
                  <TableCell className="font-mono text-xs">{failure.itemIndex + 1}</TableCell>
                  <TableCell className="text-xs text-muted-foreground font-mono">
                    {failure.itemIdentifier ?? '-'}
                  </TableCell>
                  <TableCell className="text-xs text-destructive">{failure.reason}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <Pagination
            pagination={failurePagination}
            setPagination={setFailurePagination}
            totalCount={failureTotalCount}
            isLoading={failuresQuery.isLoading}
          />
        </div>
      )}
    </div>
  )
}
