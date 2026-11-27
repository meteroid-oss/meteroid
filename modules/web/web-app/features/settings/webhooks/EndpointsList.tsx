import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { BracesIcon, MoreVerticalIcon, PlusIcon } from 'lucide-react'
import { useMemo } from 'react'

import { SettingsEmptyState } from '@/features/settings/components/SettingsSection'
import { EndpointStatusBadge, EventTypesSummary } from '@/features/settings/webhooks/badges'
import {
  lastDeliveryAt,
  needsAttention,
  outcomesOf,
  statusReason,
  successRate,
  timeAgo,
} from '@/features/settings/webhooks/endpointState'
import { useQuery } from '@/lib/connectrpc'
import { type WebhookEndpoint } from '@/rpc/api/webhooksout/v1/models_pb'
import { listWebhookEndpointStats } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'

interface Props {
  endpoints: WebhookEndpoint[]
  isLoading: boolean
  onOpen: (endpoint: WebhookEndpoint) => void
  onAdd: () => void
  onEdit: (endpoint: WebhookEndpoint) => void
  onSetUp: (endpoint: WebhookEndpoint) => void
  onToggleDisabled: (endpoint: WebhookEndpoint) => void
  onRemove: (endpoint: WebhookEndpoint) => void
}

/** Every endpoint and its health; a row opens the endpoint page. */
export const EndpointsList = ({
  endpoints,
  isLoading,
  onOpen,
  onAdd,
  onEdit,
  onSetUp,
  onToggleDisabled,
  onRemove,
}: Props) => {
  const statsQuery = useQuery(listWebhookEndpointStats, undefined, {
    enabled: endpoints.length > 0,
  })
  const rates = useMemo(
    () =>
      new Map(
        (statsQuery.data?.endpoints ?? []).map(s => [s.endpointId, successRate(outcomesOf(s))])
      ),
    [statsQuery.data]
  )

  const sorted = useMemo(
    () => [...endpoints].sort((a, b) => Number(needsAttention(b)) - Number(needsAttention(a))),
    [endpoints]
  )
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      {isLoading ? (
        <div className="space-y-2 p-4">
          <Skeleton height={20} width="100%" />
          <Skeleton height={20} width="100%" />
        </div>
      ) : endpoints.length === 0 ? (
        <SettingsEmptyState
          framed={false}
          icon={<BracesIcon />}
          title="No endpoints yet"
          action={
            <Button size="sm" variant="secondary" hasIcon onClick={onAdd}>
              <PlusIcon size={14} /> Add endpoint
            </Button>
          }
        >
          Add a URL and we&apos;ll POST a signed JSON event to it each time something happens in
          billing, like an invoice being paid.
        </SettingsEmptyState>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Endpoint</TableHead>
              <TableHead className="w-px whitespace-nowrap">Events</TableHead>
              <TableHead className="w-px whitespace-nowrap">Success, 7 days</TableHead>
              <TableHead className="w-px whitespace-nowrap">Status</TableHead>
              <TableHead className="w-px whitespace-nowrap">Last delivery</TableHead>
              <TableHead className="w-px" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {sorted.map(endpoint => {
              const reason = statusReason(endpoint)
              const last = lastDeliveryAt(endpoint)
              return (
                <TableRow
                  key={endpoint.id}
                  className="cursor-pointer"
                  onClick={() => onOpen(endpoint)}
                >
                  <TableCell className="max-w-[300px]">
                    <div className="truncate font-mono text-xs" title={endpoint.url}>
                      {endpoint.url}
                    </div>
                    {endpoint.description && (
                      <div className="truncate text-xs text-muted-foreground">
                        {endpoint.description}
                      </div>
                    )}
                  </TableCell>
                  <TableCell
                    className="w-px whitespace-nowrap text-sm text-muted-foreground"
                    onClick={e => e.stopPropagation()}
                  >
                    <EventTypesSummary eventTypes={endpoint.eventTypes} />
                  </TableCell>
                  <TableCell className="w-px whitespace-nowrap text-sm text-muted-foreground">
                    {rates.get(endpoint.id) ?? '–'}
                  </TableCell>
                  <TableCell className="w-px whitespace-nowrap">
                    <EndpointStatusBadge endpoint={endpoint} />
                    {reason && <div className="mt-0.5 text-xs text-muted-foreground">{reason}</div>}
                  </TableCell>
                  <TableCell className="w-px whitespace-nowrap text-sm text-muted-foreground">
                    {endpoint.needsSetup ? (
                      <Button
                        size="sm"
                        onClick={e => {
                          e.stopPropagation()
                          onSetUp(endpoint)
                        }}
                      >
                        Set up…
                      </Button>
                    ) : last ? (
                      timeAgo(last)
                    ) : (
                      '—'
                    )}
                  </TableCell>
                  <TableCell className="w-px" onClick={e => e.stopPropagation()}>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="sm" className="h-8 w-8 p-0">
                          <MoreVerticalIcon size={16} />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onClick={() => onEdit(endpoint)}>Edit</DropdownMenuItem>
                        {!endpoint.needsSetup && (
                          <DropdownMenuItem onClick={() => onToggleDisabled(endpoint)}>
                            {endpoint.disabled ? 'Enable' : 'Disable'}
                          </DropdownMenuItem>
                        )}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          className="text-destructive"
                          onClick={() => onRemove(endpoint)}
                        >
                          Remove
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}
    </div>
  )
}
