import {
  Alert,
  AlertDescription,
  AlertTitle,
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { AlertCircleIcon, ArrowLeftIcon, MoreHorizontalIcon } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { toast } from 'sonner'

import { DeliveriesChart } from '@/features/settings/webhooks/DeliveriesChart'
import { DeliveryLog } from '@/features/settings/webhooks/DeliveryLog'
import { DeliverySheet } from '@/features/settings/webhooks/DeliverySheet'
import { EndpointAdvanced } from '@/features/settings/webhooks/EndpointAdvanced'
import { EndpointTesting } from '@/features/settings/webhooks/EndpointTesting'
import { EndpointStatusBadge } from '@/features/settings/webhooks/badges'
import { isUnreachable, nextProbeAt } from '@/features/settings/webhooks/endpointState'
import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { useQuery } from '@/lib/connectrpc'
import {
  WebhookEndpointDisabledReasonEnum,
  type WebhookEndpoint,
} from '@/rpc/api/webhooksout/v1/models_pb'
import { getWebhookEndpointStats } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

export type EndpointTab = 'overview' | 'testing' | 'advanced'

interface Props {
  endpoint: WebhookEndpoint
  initialTab?: EndpointTab
  onBack: () => void
  onEdit: () => void
  onSetUp: () => void
  onRevealSecret: () => void
  onRotateSecret: () => void
  onToggleDisabled: () => void
  onRemove: () => void
}

/** One endpoint: its health and log, a way to test it, and its settings. */
export const EndpointPage = ({
  endpoint,
  initialTab = 'overview',
  onBack,
  onEdit,
  onSetUp,
  onRevealSecret,
  onRotateSecret,
  onToggleDisabled,
  onRemove,
}: Props) => {
  const [tab, setTab] = useState<EndpointTab>(initialTab)
  const [openDelivery, setOpenDelivery] = useState<string | null>(null)

  return (
    <div className="flex max-w-4xl flex-col gap-5">
      <div className="space-y-3">
        <Button variant="ghost" size="sm" hasIcon className="-ml-2" onClick={onBack}>
          <ArrowLeftIcon size={14} /> Webhooks
        </Button>
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0 space-y-1">
            <div className="flex items-center gap-2">
              <h3 className="truncate font-mono text-sm font-medium" title={endpoint.url}>
                {endpoint.url}
              </h3>
              <EndpointStatusBadge endpoint={endpoint} />
            </div>
            {endpoint.description && (
              <p className="text-sm text-muted-foreground">{endpoint.description}</p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <Button variant="outline" size="sm" onClick={onEdit}>
              Edit
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-8 w-8 p-0">
                  <MoreHorizontalIcon size={16} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {!endpoint.needsSetup && (
                  <DropdownMenuItem onClick={onToggleDisabled}>
                    {endpoint.disabled ? 'Enable' : 'Disable'}
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem className="text-destructive" onClick={onRemove}>
                  Remove
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </div>

      <StateBanner endpoint={endpoint} onSetUp={onSetUp} onEnable={onToggleDisabled} />

      <Tabs value={tab} onValueChange={value => setTab(value as EndpointTab)}>
        <TabsList className="justify-start">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="testing">Testing</TabsTrigger>
          <TabsTrigger value="advanced">Advanced</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="pt-4">
          <Overview endpoint={endpoint} onOpenDelivery={setOpenDelivery} />
        </TabsContent>
        <TabsContent value="testing" className="pt-4">
          <EndpointTesting endpoint={endpoint} onOpenDelivery={setOpenDelivery} />
        </TabsContent>
        <TabsContent value="advanced" className="pt-2">
          <EndpointAdvanced
            endpoint={endpoint}
            onRevealSecret={onRevealSecret}
            onRotateSecret={onRotateSecret}
            onEditHeaders={onEdit}
            onToggleDisabled={onToggleDisabled}
            onRemove={onRemove}
          />
        </TabsContent>
      </Tabs>

      <DeliverySheet
        deliveryId={openDelivery}
        endpointUrl={endpoint.url}
        onClose={() => setOpenDelivery(null)}
      />
    </div>
  )
}

/** The one thing to do about the endpoint's state, if anything. */
const StateBanner = ({
  endpoint,
  onSetUp,
  onEnable,
}: {
  endpoint: WebhookEndpoint
  onSetUp: () => void
  onEnable: () => void
}) => {
  const { resume } = useWebhooks()

  if (endpoint.needsSetup) {
    return (
      <Banner
        title="This endpoint needs setup"
        action={
          <Button size="sm" onClick={onSetUp}>
            Set up…
          </Button>
        }
      >
        A secret header value could not be copied when your webhooks were moved. Enter it to start
        receiving events here.
      </Banner>
    )
  }

  if (isUnreachable(endpoint)) {
    const next = nextProbeAt(endpoint)
    return (
      <Banner
        title="We can't reach this endpoint"
        action={
          <Button
            size="sm"
            disabled={resume.isPending}
            onClick={() =>
              resume.mutate(
                { id: endpoint.id },
                {
                  onSuccess: () => toast.success('Retrying now'),
                  onError: e => toast.error(e.message),
                }
              )
            }
          >
            Retry now
          </Button>
        }
      >
        {endpoint.failingSince
          ? `It has failed since ${parseAndFormatDateTime(endpoint.failingSince)} (${endpoint.consecutiveFailures} attempts in a row). `
          : ''}
        New events are queued, not lost.{' '}
        {next ? `We'll try again at ${parseAndFormatDateTime(next)}.` : 'We are trying again now.'}
      </Banner>
    )
  }

  const reason = endpoint.disabledReason
  if (
    endpoint.disabled &&
    (reason === WebhookEndpointDisabledReasonEnum.AUTO_FAILURES ||
      reason === WebhookEndpointDisabledReasonEnum.GONE)
  ) {
    return (
      <Banner
        title={
          reason === WebhookEndpointDisabledReasonEnum.GONE
            ? 'Your server returned 410 Gone'
            : 'Disabled after repeated failures'
        }
        action={
          <Button size="sm" onClick={onEnable}>
            Enable
          </Button>
        }
      >
        We stopped sending events to this endpoint. Enable it once it can receive them again.
      </Banner>
    )
  }

  return null
}

const Banner = ({
  title,
  action,
  children,
}: {
  title: string
  action: ReactNode
  children: ReactNode
}) => (
  <Alert variant="warning" className="flex items-start justify-between gap-4">
    <div className="flex gap-2">
      <AlertCircleIcon size={16} className="mt-0.5 shrink-0" />
      <div>
        <AlertTitle>{title}</AlertTitle>
        <AlertDescription className="text-foreground/80">{children}</AlertDescription>
      </div>
    </div>
    <div className="shrink-0">{action}</div>
  </Alert>
)

const Overview = ({
  endpoint,
  onOpenDelivery,
}: {
  endpoint: WebhookEndpoint
  onOpenDelivery: (id: string) => void
}) => {
  const statsQuery = useQuery(getWebhookEndpointStats, { endpointId: endpoint.id })

  return (
    <div className="space-y-6">
      <dl className="grid grid-cols-2 gap-x-6 gap-y-3 text-[13px] sm:grid-cols-4">
        <Fact label="Events">
          {endpoint.eventTypes.length === 0
            ? 'All events'
            : `${endpoint.eventTypes.length} event type${endpoint.eventTypes.length === 1 ? '' : 's'}`}
        </Fact>
        <Fact label="Last success">
          {endpoint.lastSuccessAt ? parseAndFormatDateTime(endpoint.lastSuccessAt) : '—'}
        </Fact>
        <Fact label="Last failure">
          {endpoint.lastFailureAt ? parseAndFormatDateTime(endpoint.lastFailureAt) : '—'}
        </Fact>
        <Fact label="Created">{parseAndFormatDateTime(endpoint.createdAt)}</Fact>
      </dl>

      {endpoint.eventTypes.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {endpoint.eventTypes.map(type => (
            <Badge key={type} variant="outline" size="sm" className="font-mono">
              {type}
            </Badge>
          ))}
        </div>
      )}

      <section className="space-y-3 rounded-lg border border-border p-4">
        <h3 className="text-sm font-medium">Deliveries · last 7 days</h3>
        <DeliveriesChart stats={statsQuery.data?.days ?? []} />
      </section>

      <DeliveryLog endpointId={endpoint.id} onOpen={onOpenDelivery} />
    </div>
  )
}

const Fact = ({ label, children }: { label: string; children: ReactNode }) => (
  <div>
    <dt className="text-xs text-muted-foreground">{label}</dt>
    <dd className="mt-0.5">{children}</dd>
  </div>
)
