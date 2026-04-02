import { createConnectQueryKey } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Checkbox,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { RefreshCwIcon, XCircleIcon } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'

import PageHeading from '@/components/PageHeading/PageHeading'
import { StandardTable } from '@/components/table/StandardTable'
import { OrgTenantFilterSelect } from '@/features/admin/deadletter/OrgTenantFilter'
import { getStatusConfig, QUEUE_OPTIONS } from '@/features/admin/deadletter/statusConfig'
import { useMutation, useQuery } from '@/lib/connectrpc'
import {
  batchDiscard,
  batchRequeue,
  getQueueHealth,
  listDeadLetters,
} from '@/rpc/admin/deadletter/v1/deadletter-DeadLetterService_connectquery'
import { DeadLetterEntry, DeadLetterStatus } from '@/rpc/admin/deadletter/v1/deadletter_pb'
import { parseAndFormatDateTime } from '@/utils/date'

export const DeadLetterList = () => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 15,
  })
  const [queueFilter, setQueueFilter] = useState<string>('all')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [orgFilter, setOrgFilter] = useState<string | undefined>()
  const [tenantFilter, setTenantFilter] = useState<string | undefined>()
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const queryClient = useQueryClient()

  const listQuery = useQuery(listDeadLetters, {
    queue: queueFilter === 'all' ? undefined : queueFilter,
    status:
      statusFilter === 'all'
        ? DeadLetterStatus.UNSPECIFIED
        : (Number(statusFilter) as DeadLetterStatus),
    organizationId: orgFilter,
    tenantId: tenantFilter,
    pagination: {
      page: pagination.pageIndex,
      perPage: pagination.pageSize,
    },
  })

  const healthQuery = useQuery(getQueueHealth)

  useEffect(() => {
    setPagination(prev => ({ ...prev, pageIndex: 0 }))
  }, [queueFilter, statusFilter, orgFilter, tenantFilter])

  const invalidate = async () => {
    await queryClient.invalidateQueries({
      queryKey: createConnectQueryKey(listDeadLetters),
    })
    await queryClient.invalidateQueries({
      queryKey: createConnectQueryKey(getQueueHealth),
    })
    setSelectedIds(new Set())
  }

  const batchRequeueMut = useMutation(batchRequeue, {
    onSuccess: async res => {
      await invalidate()
      toast.success(`Requeued ${res.requeuedCount} message(s)`)
    },
    onError: err => toast.error(`Batch requeue failed: ${err.message}`),
  })

  const batchDiscardMut = useMutation(batchDiscard, {
    onSuccess: async res => {
      await invalidate()
      toast.success(`Discarded ${res.discardedCount} message(s)`)
    },
    onError: err => toast.error(`Batch discard failed: ${err.message}`),
  })

  const entries = listQuery.data?.entries ?? []
  const totalCount = listQuery.data?.paginationMeta?.totalItems ?? 0
  const totalPending =
    healthQuery.data?.queues?.reduce((sum, q) => sum + Number(q.pendingCount), 0) ?? 0

  const pendingEntries = entries.filter(e => e.status === DeadLetterStatus.PENDING)
  const allPendingSelected =
    pendingEntries.length > 0 && pendingEntries.every(e => selectedIds.has(e.id))

  const toggleSelect = (id: string) => {
    setSelectedIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const toggleAll = () => {
    if (allPendingSelected) {
      setSelectedIds(new Set())
    } else {
      setSelectedIds(new Set(pendingEntries.map(e => e.id)))
    }
  }

  const columns = useMemo<ColumnDef<DeadLetterEntry>[]>(
    () => [
      {
        id: 'select',
        header: () => (
          <Checkbox
            checked={allPendingSelected && pendingEntries.length > 0}
            onCheckedChange={toggleAll}
          />
        ),
        cell: ({ row }) =>
          row.original.status === DeadLetterStatus.PENDING ? (
            <div onClick={e => e.stopPropagation()}>
              <Checkbox
                checked={selectedIds.has(row.original.id)}
                onCheckedChange={() => toggleSelect(row.original.id)}
              />
            </div>
          ) : null,
        enableSorting: false,
        className: 'w-10',
      },
      {
        header: 'Queue',
        accessorKey: 'queue',
        cell: ({ row }) => <span className="font-mono text-sm">{row.original.queue}</span>,
      },
      {
        header: 'Organization',
        id: 'organization',
        enableSorting: false,
        cell: ({ row }) => {
          const { organizationName } = row.original
          if (!organizationName) return <span className="text-sm text-muted-foreground">-</span>
          return <span className="text-sm">{organizationName}</span>
        },
      },
      {
        header: 'Status',
        accessorKey: 'status',
        enableSorting: false,
        cell: ({ row }) => {
          const { label, variant } = getStatusConfig(row.original.status)
          return <Badge variant={variant}>{label}</Badge>
        },
      },
      {
        header: 'Last Error',
        accessorKey: 'lastError',
        enableSorting: false,
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground max-w-[300px] truncate block">
            {row.original.lastError ?? '-'}
          </span>
        ),
      },
      {
        header: 'Retries',
        accessorKey: 'readCount',
        enableSorting: false,
      },
      {
        header: 'Dead-lettered',
        accessorKey: 'deadLetteredAt',
        cell: ({ row }) => (
          <span className="text-sm text-muted-foreground">
            {parseAndFormatDateTime(row.original.deadLetteredAt)}
          </span>
        ),
      },
    ],
    [allPendingSelected, pendingEntries, selectedIds]
  )

  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-row items-center justify-between">
        <PageHeading count={totalCount}>Dead Letter Queue</PageHeading>
        {selectedIds.size > 0 && (
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">{selectedIds.size} selected</span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => batchRequeueMut.mutate({ ids: [...selectedIds] })}
              disabled={batchRequeueMut.isPending}
            >
              <RefreshCwIcon size={14} className="mr-1" />
              Requeue
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => batchDiscardMut.mutate({ ids: [...selectedIds] })}
              disabled={batchDiscardMut.isPending}
            >
              <XCircleIcon size={14} className="mr-1" />
              Discard
            </Button>
          </div>
        )}
      </div>

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

      <div className="flex flex-row items-center gap-2 flex-wrap">
        <OrgTenantFilterSelect
          organizationId={orgFilter}
          tenantId={tenantFilter}
          onOrganizationChange={setOrgFilter}
          onTenantChange={setTenantFilter}
        />

        <Select value={queueFilter} onValueChange={setQueueFilter}>
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

        <Select value={statusFilter} onValueChange={setStatusFilter}>
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

        <Button
          variant="outline"
          size="sm"
          disabled={listQuery.isLoading}
          onClick={() => listQuery.refetch()}
        >
          <RefreshCwIcon size={14} className={listQuery.isLoading ? 'animate-spin' : ''} />
        </Button>
      </div>

      <StandardTable
        columns={columns}
        data={entries}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={listQuery.isLoading}
        emptyMessage={
          totalPending > 0
            ? `${totalPending} pending message(s) across all queues. Adjust filters to see them.`
            : 'No dead-lettered messages found'
        }
        rowLink={row => row.original.id}
      />
    </div>
  )
}
