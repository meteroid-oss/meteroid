import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Card,
  CardContent,
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
import {
  ArrowLeftIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  DownloadIcon,
  FileIcon,
  RefreshCwIcon,
  XCircleIcon,
} from 'lucide-react'
import { Fragment, FunctionComponent, useCallback, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { BatchProgressBar } from '@/features/batch-jobs/BatchProgressBar'
import {
  ACTIVE_STATUSES,
  BatchJobChunkStatus,
  BatchJobStatus,
  CHUNK_EVENT_CONFIG,
  JOB_TYPE_LABELS,
  getChunkStatusConfig,
  getStatusConfig,
} from '@/features/batch-jobs/statusConfig'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import {
  cancelBatchJob,
  getBatchJob,
  listBatchJobFailures,
  retryBatchJob,
} from '@/rpc/api/batchjobs/v1/batchjobs-BatchJobsService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

function formatDuration(start: Date, end: Date): string {
  const secs = Math.round((end.getTime() - start.getTime()) / 1000)
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  const remSecs = secs % 60
  if (mins < 60) return `${mins}m ${remSecs}s`
  const hrs = Math.floor(mins / 60)
  const remMins = mins % 60
  return `${hrs}h ${remMins}m`
}

interface BatchJobDetailProps {
  jobId: string
  onBack?: () => void
}

const BackToJobs: FunctionComponent<{ onBack?: () => void }> = ({ onBack }) =>
  onBack ? (
    <Button variant="ghost" size="sm" onClick={onBack}>
      <ArrowLeftIcon size={14} className="mr-1" /> Back to jobs
    </Button>
  ) : (
    <Button variant="ghost" size="sm" asChild>
      <Link to="..?tab=batch-jobs">
        <ArrowLeftIcon size={14} className="mr-1" /> Back to jobs
      </Link>
    </Button>
  )

export const BatchJobDetail: FunctionComponent<BatchJobDetailProps> = ({ jobId, onBack }) => {
  const queryClient = useQueryClient()
  const [showCancelDialog, setShowCancelDialog] = useState(false)
  const [expandedChunks, setExpandedChunks] = useState<Set<string>>(new Set())
  const [failurePagination, setFailurePagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 15,
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

  const jobStatus = jobQuery.data?.job?.status
  const isJobActive = jobStatus !== undefined && ACTIVE_STATUSES.includes(jobStatus)

  const failuresQuery = useQuery(
    listBatchJobFailures,
    {
      jobId,
      limit: failurePagination.pageSize,
      offset: failurePagination.pageIndex * failurePagination.pageSize,
    },
    {
      refetchInterval: isJobActive ? 5000 : false,
    }
  )

  const hasRetried = useRef(false)

  const cancelMut = useMutation(cancelBatchJob, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getBatchJob, { jobId: jobId }),
      })
      toast.success('Job cancelled')
      setShowCancelDialog(false)
    },
    onError: error => {
      toast.error(`Failed to cancel job: ${error.message}`)
    },
  })

  const retryMut = useMutation(retryBatchJob, {
    onSuccess: async res => {
      await new Promise(resolve => setTimeout(resolve, 1000))

      await queryClient.invalidateQueries({ queryKey: [getBatchJob.service.typeName] })
      hasRetried.current = true
      toast.success(`Retrying ${res.retriedChunks} batch(es)`)
    },
    onError: error => {
      toast.error(`Failed to retry: ${error.message}`)
    },
  })

  const handleDownloadErrors = useCallback(() => {
    const token = jobQuery.data?.job?.errorCsvToken
    if (!token) return
    const link = document.createElement('a')
    link.href = `${env.meteroidRestApiUri}/files/v1/batch-job/errors/${jobId}?token=${token}`
    link.download = `errors-${jobId}.csv`
    link.click()
  }, [jobId, jobQuery.data?.job?.errorCsvToken])

  const handleDownloadSourceFile = useCallback(() => {
    const token = jobQuery.data?.job?.inputFileToken
    if (!token) return
    const link = document.createElement('a')
    link.href = `${env.meteroidRestApiUri}/files/v1/batch-job/input/${jobId}?token=${token}`
    link.download = jobQuery.data?.job?.inputFileName ?? `input-${jobId}.csv`
    link.click()
  }, [jobId, jobQuery.data?.job?.inputFileToken, jobQuery.data?.job?.inputFileName])

  if (jobQuery.isLoading) {
    return (
      <div className="py-4 space-y-4">
        <Skeleton height={200} />
      </div>
    )
  }

  const job = jobQuery.data?.job
  const chunks = jobQuery.data?.chunks ?? []
  const failures = failuresQuery.data?.failures ?? []
  const failureTotalCount = Number(failuresQuery.data?.totalCount ?? 0)

  if (!job) {
    return (
      <div className="py-4">
        <div className="mb-4">
          <BackToJobs onBack={onBack} />
        </div>
        <div className="border border-border rounded-lg p-8 text-center">
          <p className="text-sm text-muted-foreground">Job not found</p>
        </div>
      </div>
    )
  }

  const { label: statusLabel, variant: statusVariant } = getStatusConfig(job.status)
  const isActive = isJobActive
  const canCancel = isActive
  const canRetry = !hasRetried.current && job.status === BatchJobStatus.FAILED
  const total = job.totalItems ?? 0
  const progressPct = total > 0 ? Math.round((job.processedItems / total) * 100) : 0

  return (
    <div className="py-4 space-y-4">
      <BackToJobs onBack={onBack} />

      {/* Job Summary */}
      <Card>
        <CardContent className="p-6 space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-lg font-semibold">
                {JOB_TYPE_LABELS[job.jobType] ?? String(job.jobType)}
              </h2>
              <p className="text-xs text-muted-foreground font-mono">{job.id}</p>
            </div>
            <div className="flex items-center gap-2">
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
              {canCancel && (
                <Button variant="destructive" size="sm" onClick={() => setShowCancelDialog(true)}>
                  <XCircleIcon size={14} className="mr-1" />
                  Cancel
                </Button>
              )}
            </div>
          </div>

          <div className="grid grid-cols-2 md:grid-cols-5 gap-4 text-sm">
            <div>
              <span className="text-muted-foreground">Status</span>
              <div className="mt-1">
                <Badge variant={statusVariant}>{statusLabel}</Badge>
              </div>
            </div>
            <div>
              <span className="text-muted-foreground">Processed</span>
              <div className="mt-1 font-mono">
                {job.processedItems} / {job.totalItems ?? '?'}
              </div>
            </div>
            <div>
              <span className="text-muted-foreground">Failed</span>
              <div className="mt-1">
                {job.failedItems > 0 ? (
                  <span className="text-destructive font-medium">{job.failedItems}</span>
                ) : (
                  '0'
                )}
              </div>
            </div>
            <div>
              <span className="text-muted-foreground">Started</span>
              <div className="mt-1">{parseAndFormatDateTime(job.createdAt)}</div>
            </div>
            <div>
              <span className="text-muted-foreground">Duration</span>
              <div className="mt-1">
                {job.completedAt
                  ? formatDuration(new Date(job.createdAt), new Date(job.completedAt))
                  : isActive
                    ? 'In progress…'
                    : '-'}
              </div>
            </div>
          </div>

          {(job.inputFileName || job.createdByDisplayName) && (
            <div className="grid grid-cols-2 md:grid-cols-5 gap-4 text-sm">
              {job.inputFileName && (
                <div>
                  <span className="text-muted-foreground">Source file</span>
                  <div className="mt-1 flex items-center gap-1.5">
                    <FileIcon size={14} className="text-muted-foreground shrink-0" />
                    <span className="truncate">{job.inputFileName}</span>
                    {job.inputFileToken && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-5 w-5 shrink-0"
                        onClick={handleDownloadSourceFile}
                      >
                        <DownloadIcon size={12} />
                      </Button>
                    )}
                  </div>
                </div>
              )}
              {job.createdByDisplayName && (
                <div>
                  <span className="text-muted-foreground">Uploaded by</span>
                  <div className="mt-1">{job.createdByDisplayName}</div>
                </div>
              )}
            </div>
          )}

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

          {job.errorMessage && (
            <div className="flex items-start gap-2 p-3 border border-destructive/50 rounded-md bg-destructive/5">
              <XCircleIcon size={16} className="text-destructive mt-0.5 shrink-0" />
              <p className="text-sm text-destructive">{job.errorMessage}</p>
            </div>
          )}

          {/* Fix & re-import section */}
          {!isActive && job.errorCsvToken && (
            <div className="border-t border-border pt-4 space-y-2">
              <h4 className="text-sm font-medium">Fix & re-import</h4>
              <Button variant="outline" size="sm" onClick={handleDownloadErrors}>
                <DownloadIcon size={14} className="mr-1" />
                Download failed rows
              </Button>
              <p className="text-xs text-muted-foreground">
                CSV containing only the failed rows with an additional _error column. Fix the errors
                and re-import.
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Chunks */}
      {chunks.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">Batches ({chunks.length})</h3>
          <Table containerClassName="border border-border rounded-lg max-h-[400px] overflow-y-auto">
            <TableHeader>
              <TableRow>
                <TableHead className="w-[40px]"></TableHead>
                <TableHead className="w-[60px]">#</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Rows</TableHead>
                <TableHead>Processed</TableHead>
                <TableHead>Failed</TableHead>
                <TableHead>Retries</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {chunks.map(chunk => {
                const chunkConfig = getChunkStatusConfig(chunk.status)
                const hasFailures = chunk.failedCount > 0
                const badgeVariant =
                  chunk.status === BatchJobChunkStatus.CHUNK_COMPLETED && hasFailures
                    ? 'warning'
                    : chunkConfig.variant
                const badgeLabel =
                  chunk.status === BatchJobChunkStatus.CHUNK_COMPLETED && hasFailures
                    ? 'Partial'
                    : chunkConfig.label
                const isExpanded = expandedChunks.has(chunk.id)
                const toggleExpand = () => {
                  setExpandedChunks(prev => {
                    const next = new Set(prev)
                    if (next.has(chunk.id)) next.delete(chunk.id)
                    else next.add(chunk.id)
                    return next
                  })
                }
                return (
                  <Fragment key={chunk.id}>
                    <TableRow
                      className={chunk.events.length > 0 ? 'cursor-pointer' : ''}
                      onClick={chunk.events.length > 0 ? toggleExpand : undefined}
                    >
                      <TableCell>
                        {chunk.events.length > 0 &&
                          (isExpanded ? (
                            <ChevronUpIcon size={14} className="text-muted-foreground" />
                          ) : (
                            <ChevronDownIcon size={14} className="text-muted-foreground" />
                          ))}
                      </TableCell>
                      <TableCell className="font-mono text-xs">{chunk.chunkIndex + 1}</TableCell>
                      <TableCell>
                        <Badge variant={badgeVariant}>{badgeLabel}</Badge>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground font-mono">
                        {chunk.itemOffset + 1}–{chunk.itemOffset + chunk.itemCount}
                      </TableCell>
                      <TableCell className="text-xs font-mono">{chunk.processedCount}</TableCell>
                      <TableCell className="text-xs">
                        {chunk.failedCount > 0 ? (
                          <span className="text-destructive font-medium">{chunk.failedCount}</span>
                        ) : (
                          '0'
                        )}
                      </TableCell>
                      <TableCell className="text-xs font-mono">
                        {chunk.retryCount > 0 ? (
                          <span className="text-destructive font-medium">{chunk.retryCount}</span>
                        ) : (
                          '-'
                        )}
                      </TableCell>
                    </TableRow>
                    {isExpanded && chunk.events.length > 0 && (
                      <TableRow>
                        <TableCell colSpan={7} className="bg-muted/30 px-8 py-3">
                          <div className="space-y-1.5">
                            {chunk.events.map((evt, idx) => {
                              const evtConfig = CHUNK_EVENT_CONFIG[evt.event] ?? {
                                label: evt.event,
                                variant: 'secondary' as const,
                              }
                              const dotColorMap: Record<string, string> = {
                                success: 'bg-success',
                                destructive: 'bg-destructive',
                                warning: 'bg-warning',
                                default: 'bg-brand',
                                secondary: 'bg-muted-foreground',
                              }
                              const dotColor =
                                dotColorMap[evtConfig.variant] ?? 'bg-muted-foreground'
                              return (
                                <div key={idx} className="flex items-start gap-2 text-xs">
                                  <div
                                    className={`w-1.5 h-1.5 rounded-full ${dotColor} mt-1.5 shrink-0`}
                                  />
                                  <div className="flex-1 min-w-0 flex items-center gap-2">
                                    <span className="font-medium">{evtConfig.label}</span>
                                    <span className="text-muted-foreground">
                                      attempt {evt.attempt}
                                    </span>
                                    {evt.message && (
                                      <span className="text-muted-foreground truncate">
                                        {evt.message}
                                      </span>
                                    )}
                                    <span className="text-muted-foreground ml-auto shrink-0">
                                      {parseAndFormatDateTime(evt.timestamp)}
                                    </span>
                                  </div>
                                </div>
                              )
                            })}
                          </div>
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                )
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Failures Table */}
      {failureTotalCount > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">Failures ({failureTotalCount})</h3>
          <Table containerClassName="border border-border rounded-lg">
            <TableHeader>
              <TableRow>
                <TableHead className="w-[80px]">Row</TableHead>
                <TableHead className="w-[180px]">Identifier</TableHead>
                <TableHead>Error</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {failures.map(failure => (
                <TableRow key={failure.id}>
                  <TableCell className="font-mono text-sm">{failure.itemIndex + 1}</TableCell>
                  <TableCell className="text-sm text-muted-foreground font-mono">
                    {failure.itemIdentifier ?? '-'}
                  </TableCell>
                  <TableCell className="text-sm text-destructive">{failure.reason}</TableCell>
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

      {/* Cancel Confirmation */}
      <AlertDialog open={showCancelDialog} onOpenChange={setShowCancelDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Cancel Batch Job?</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to cancel this job? Items already processed will not be
              reverted, but remaining items will not be processed.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep Running</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => cancelMut.mutate({ jobId })}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Cancel Job
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
