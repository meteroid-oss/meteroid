import {
  Button,
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
import { RefreshCwIcon } from 'lucide-react'
import { useState } from 'react'

import Pagination from '@/components/table/CustomTable/components/Pagination/Pagination'
import { DELIVERY_STATUS_LABEL, DeliveryStatusBadge } from '@/features/settings/webhooks/badges'
import { useQuery } from '@/lib/connectrpc'
import { WebhookDeliveryStatusEnum } from '@/rpc/api/webhooksout/v1/models_pb'
import { listWebhookDeliveries } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

const ALL_STATUSES = 'ALL'
const PER_PAGE = 20

const STATUS_OPTIONS = [
  WebhookDeliveryStatusEnum.PENDING,
  WebhookDeliveryStatusEnum.IN_FLIGHT,
  WebhookDeliveryStatusEnum.SUCCEEDED,
  WebhookDeliveryStatusEnum.FAILED,
  WebhookDeliveryStatusEnum.CANCELLED,
]

interface Props {
  endpointId: string
  onOpen: (deliveryId: string) => void
}

/** Every event sent to one endpoint, newest first; a row opens the delivery sheet. */
export const DeliveryLog = ({ endpointId, onOpen }: Props) => {
  const [status, setStatus] = useState<string>(ALL_STATUSES)
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: PER_PAGE,
  })

  const deliveriesQuery = useQuery(listWebhookDeliveries, {
    endpointId,
    status: status === ALL_STATUSES ? undefined : (Number(status) as WebhookDeliveryStatusEnum),
    pagination: { page: pagination.pageIndex, perPage: pagination.pageSize },
  })

  const deliveries = deliveriesQuery.data?.deliveries ?? []
  const totalCount = deliveriesQuery.data?.paginationMeta?.totalItems ?? 0

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-medium">Delivery log</h3>
        <div className="flex items-center gap-2">
          <Select
            value={status}
            onValueChange={value => {
              setStatus(value)
              setPagination(prev => ({ ...prev, pageIndex: 0 }))
            }}
          >
            <SelectTrigger className="h-8 w-[160px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL_STATUSES}>All statuses</SelectItem>
              {STATUS_OPTIONS.map(option => (
                <SelectItem key={option} value={String(option)}>
                  {DELIVERY_STATUS_LABEL[option]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            variant="ghost"
            size="sm"
            hasIcon
            onClick={() => deliveriesQuery.refetch()}
            disabled={deliveriesQuery.isFetching}
          >
            <RefreshCwIcon size={14} /> Refresh
          </Button>
        </div>
      </div>

      <div className="overflow-hidden rounded-lg border border-border">
        {deliveriesQuery.isLoading ? (
          <div className="space-y-2 p-4">
            <Skeleton height={20} width="100%" />
            <Skeleton height={20} width="100%" />
          </div>
        ) : deliveries.length === 0 ? (
          <p className="py-10 text-center text-sm text-muted-foreground">No deliveries yet.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Event</TableHead>
                <TableHead className="w-px whitespace-nowrap">Status</TableHead>
                <TableHead className="w-px whitespace-nowrap">Attempts</TableHead>
                <TableHead className="w-px whitespace-nowrap">Response</TableHead>
                <TableHead className="w-px whitespace-nowrap">Created</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {deliveries.map(delivery => (
                <TableRow
                  key={delivery.id}
                  className="cursor-pointer"
                  onClick={() => onOpen(delivery.id)}
                >
                  <TableCell className="max-w-[260px] truncate font-mono text-xs">
                    {delivery.eventType}
                    {delivery.manual && (
                      <span className="ml-1.5 text-[10.5px] uppercase tracking-wide text-muted-foreground">
                        {delivery.eventType === 'webhook.test' ? 'test' : 'manual'}
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="w-px">
                    <DeliveryStatusBadge status={delivery.status} />
                  </TableCell>
                  <TableCell className="w-px text-center text-sm text-muted-foreground">
                    {Number(delivery.attempts)}
                  </TableCell>
                  <TableCell className="w-px text-center text-sm text-muted-foreground">
                    {delivery.lastResponseStatus ?? '—'}
                  </TableCell>
                  <TableCell className="w-px whitespace-nowrap text-sm text-muted-foreground">
                    {parseAndFormatDateTime(delivery.createdAt)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>

      <Pagination
        pagination={pagination}
        setPagination={setPagination}
        totalCount={totalCount}
        isLoading={deliveriesQuery.isLoading}
      />
    </section>
  )
}
