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
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { getStatusConfig, QUEUE_OPTIONS } from '@/features/admin/deadletter/statusConfig'
import { useQuery } from '@/lib/connectrpc'
import {
  listDeadLetters,
  getQueueHealth,
} from '@/rpc/admin/deadletter/v1/deadletter-DeadLetterService_connectquery'
import { DeadLetterStatus } from '@/rpc/admin/deadletter/v1/deadletter_pb'
import { parseAndFormatDateTime } from '@/utils/date'

export const DeadLetterList = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 25,
  })
  const [queueFilter, setQueueFilter] = useState<string>('all')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const navigate = useNavigate()

  const listQuery = useQuery(listDeadLetters, {
    queue: queueFilter === 'all' ? undefined : queueFilter,
    status:
      statusFilter === 'all'
        ? DeadLetterStatus.UNSPECIFIED
        : (Number(statusFilter) as DeadLetterStatus),
    limit: pagination.pageSize,
    offset: pagination.pageIndex * pagination.pageSize,
  })

  const healthQuery = useQuery(getQueueHealth)

  const entries = listQuery.data?.entries ?? []
  const totalCount = Number(listQuery.data?.totalCount ?? 0)
  const totalPending =
    healthQuery.data?.queues?.reduce((sum, q) => sum + Number(q.pendingCount), 0) ?? 0

  if (listQuery.isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton height={200} />
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-lg pb-1 font-semibold">Dead Letter Queue</h1>
        <p className="text-sm text-muted-foreground">
          {totalPending > 0
            ? `${totalPending} message(s) pending review across all queues.`
            : 'No pending dead-lettered messages.'}
        </p>
      </div>

      {/* Queue health summary */}
      {healthQuery.data?.queues && healthQuery.data.queues.length > 0 && (
        <div className="flex gap-2 flex-wrap">
          {healthQuery.data.queues
            .filter(q => Number(q.pendingCount) > 0)
            .map(q => (
              <Badge key={q.queue} variant="destructive" className="text-xs">
                {q.queue}: {String(q.pendingCount)}
              </Badge>
            ))}
        </div>
      )}

      <div className="flex items-center gap-4">
        <Select
          value={queueFilter}
          onValueChange={v => {
            setQueueFilter(v)
            setPagination(p => ({ ...p, pageIndex: 0 }))
          }}
        >
          <SelectTrigger className="w-[220px]">
            <SelectValue placeholder="Filter by queue" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All queues</SelectItem>
            {QUEUE_OPTIONS.map(q => (
              <SelectItem key={q} value={q}>
                {q}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select
          value={statusFilter}
          onValueChange={v => {
            setStatusFilter(v)
            setPagination(p => ({ ...p, pageIndex: 0 }))
          }}
        >
          <SelectTrigger className="w-[180px]">
            <SelectValue placeholder="Filter by status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All statuses</SelectItem>
            <SelectItem value={String(DeadLetterStatus.PENDING)}>Pending</SelectItem>
            <SelectItem value={String(DeadLetterStatus.REQUEUED)}>Requeued</SelectItem>
            <SelectItem value={String(DeadLetterStatus.DISCARDED)}>Discarded</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {entries.length === 0 ? (
        <div className="border border-border rounded-lg p-8 text-center">
          <p className="text-sm text-muted-foreground">No dead-lettered messages found</p>
        </div>
      ) : (
        <Table containerClassName="border border-border rounded-lg">
          <TableHeader>
            <TableRow>
              <TableHead>Queue</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Last Error</TableHead>
              <TableHead>Retries</TableHead>
              <TableHead>Enqueued</TableHead>
              <TableHead>Dead-lettered</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {entries.map(entry => {
              const { label, variant } = getStatusConfig(entry.status)
              return (
                <TableRow
                  key={entry.id}
                  className="cursor-pointer"
                  onClick={() => navigate(entry.id)}
                >
                  <TableCell className="font-mono text-sm">{entry.queue}</TableCell>
                  <TableCell>
                    <Badge variant={variant}>{label}</Badge>
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground max-w-[300px] truncate">
                    {entry.lastError ?? '-'}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">{entry.readCount}</TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {parseAndFormatDateTime(entry.enqueuedAt)}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {parseAndFormatDateTime(entry.deadLetteredAt)}
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}

      <Pagination
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={listQuery.isLoading}
      />
    </div>
  )
}
