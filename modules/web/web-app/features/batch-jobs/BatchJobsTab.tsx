import {
  Badge,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { FunctionComponent, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import {
  ACTIVE_STATUSES,
  BatchJobStatus,
  BatchJobType,
  JOB_TYPE_LABELS,
  getStatusConfig,
} from '@/features/batch-jobs/statusConfig'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listBatchJobs } from '@/rpc/api/batchjobs/v1/batchjobs-BatchJobsService_connectquery'
import { BatchJob } from '@/rpc/api/batchjobs/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

function formatJobType(jobType: BatchJobType) {
  return JOB_TYPE_LABELS[jobType] ?? String(jobType)
}

function formatProgress(job: BatchJob) {
  const total = job.totalItems ?? '?'
  return `${job.processedItems} / ${total}`
}

export const BatchJobsTab: FunctionComponent = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 25,
  })
  const [jobTypeFilter, setJobTypeFilter] = useState<string>('all')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const navigate = useNavigate()
  const basePath = useBasePath()

  const jobsQuery = useQuery(
    listBatchJobs,
    {
      jobType: jobTypeFilter === 'all' ? undefined : (Number(jobTypeFilter) as BatchJobType),
      statuses: statusFilter === 'all' ? [] : [Number(statusFilter) as BatchJobStatus],
      limit: pagination.pageSize,
      offset: pagination.pageIndex * pagination.pageSize,
    },
    {
      refetchInterval: data => {
        const hasActive = data?.state?.data?.jobs?.some(j => ACTIVE_STATUSES.includes(j.status))
        return hasActive ? 5000 : false
      },
    }
  )

  const jobs = jobsQuery.data?.jobs ?? []
  const totalCount = Number(jobsQuery.data?.totalCount ?? 0)

  if (jobsQuery.isLoading) {
    return (
      <div className="flex max-w-3xl flex-col gap-6">
        <Skeleton height={200} />
      </div>
    )
  }

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <SettingsSection
        divided={false}
        title="Batch jobs"
        description="Monitor and manage asynchronous batch processing jobs like CSV imports."
      >
        <div className="space-y-4">
          <div className="flex items-center gap-3">
            <Select
              value={jobTypeFilter}
              onValueChange={v => {
                setJobTypeFilter(v)
                setPagination(p => ({ ...p, pageIndex: 0 }))
              }}
            >
              <SelectTrigger className="h-8 w-[200px]">
                <SelectValue placeholder="Filter by type" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All types</SelectItem>
                <SelectItem value={String(BatchJobType.EVENT_CSV_IMPORT)}>Event Import</SelectItem>
                <SelectItem value={String(BatchJobType.CUSTOMER_CSV_IMPORT)}>
                  Customer Import
                </SelectItem>
                <SelectItem value={String(BatchJobType.SUBSCRIPTION_CSV_IMPORT)}>
                  Subscription Import
                </SelectItem>
                <SelectItem value={String(BatchJobType.SUBSCRIPTION_PLAN_MIGRATION)}>
                  Plan Migration
                </SelectItem>
              </SelectContent>
            </Select>

            <Select
              value={statusFilter}
              onValueChange={v => {
                setStatusFilter(v)
                setPagination(p => ({ ...p, pageIndex: 0 }))
              }}
            >
              <SelectTrigger className="h-8 w-[200px]">
                <SelectValue placeholder="Filter by status" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All statuses</SelectItem>
                <SelectItem value={String(BatchJobStatus.PENDING)}>Pending</SelectItem>
                <SelectItem value={String(BatchJobStatus.CHUNKING)}>Chunking</SelectItem>
                <SelectItem value={String(BatchJobStatus.PROCESSING)}>Processing</SelectItem>
                <SelectItem value={String(BatchJobStatus.COMPLETED)}>Completed</SelectItem>
                <SelectItem value={String(BatchJobStatus.COMPLETED_WITH_ERRORS)}>
                  Completed with Errors
                </SelectItem>
                <SelectItem value={String(BatchJobStatus.FAILED)}>Failed</SelectItem>
                <SelectItem value={String(BatchJobStatus.CANCELLED)}>Cancelled</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {jobs.length === 0 ? (
            <div className="rounded-lg border border-border p-8 text-center">
              <p className="text-sm text-muted-foreground">No batch jobs found</p>
            </div>
          ) : (
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
              <TableRow>
                <TableHead>Job Type</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>File</TableHead>
                <TableHead>Progress</TableHead>
                <TableHead>Failed</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>Completed</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.map(job => {
                const { label, variant } = getStatusConfig(job.status)
                const hasPartialFailures =
                  job.status === BatchJobStatus.COMPLETED && job.failedItems > 0
                const badgeVariant = hasPartialFailures ? 'warning' : variant
                const badgeLabel = hasPartialFailures ? 'Completed (Partial)' : label
                return (
                  <TableRow
                    key={job.id}
                    className="cursor-pointer"
                    onClick={() => navigate(`${basePath}/developers/batch-jobs/${job.id}`)}
                  >
                    <TableCell className="font-medium">{formatJobType(job.jobType)}</TableCell>
                    <TableCell>
                      <Badge variant={badgeVariant}>{badgeLabel}</Badge>
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground font-mono">
                      {job.inputFileName}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground font-mono">
                      {formatProgress(job)}
                    </TableCell>

                    <TableCell className="text-sm text-muted-foreground">
                      {job.failedItems > 0 ? (
                        <span className="text-destructive font-medium">{job.failedItems}</span>
                      ) : (
                        '0'
                      )}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {parseAndFormatDateTime(job.createdAt)}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {job.completedAt ? parseAndFormatDateTime(job.completedAt) : '-'}
                    </TableCell>
                  </TableRow>
                )
              })}
                </TableBody>
              </Table>
            </div>
          )}
          <Pagination
            pagination={pagination}
            setPagination={setPagination}
            totalCount={totalCount}
            isLoading={jobsQuery.isLoading}
          />
        </div>
      </SettingsSection>
    </div>
  )
}
