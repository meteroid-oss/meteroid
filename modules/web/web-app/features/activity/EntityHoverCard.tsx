import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  Skeleton,
} from '@md/ui'
import { Link } from 'react-router-dom'

import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

type CommonProps = {
  entityType: string
  entityId: string
  label?: string
  className?: string
}

// `null` route = render as plain text, no link.
const ROUTE_FOR: Record<string, string | null> = {
  customer: 'customers',
  subscription: 'subscriptions',
  invoice: 'invoices',
  credit_note: 'credit-notes',
  quote: 'quotes',
  plan: 'plans',
  product: 'products',
  add_on: 'add-ons',
  coupon: 'coupons',
  billable_metric: 'billable-metrics',
  payment_transaction: null,
  api_token: null,
  user: null,
  connector: null,
  webhook_endpoint: null,
  invoicing_entity: null,
  tenant: null,
}

export const EntityHoverCard = ({ entityType, entityId, label, className }: CommonProps) => {
  const basePath = useBasePath()
  const route = ROUTE_FOR[entityType]
  const text = label ?? defaultLabel(entityType, entityId)

  const trigger = route ? (
    <Link
      to={`${basePath}/${route}/${entityId}`}
      className={`text-foreground underline-offset-2 hover:underline ${className ?? ''}`}
    >
      {text}
    </Link>
  ) : (
    <span className={`text-foreground ${className ?? ''}`}>{text}</span>
  )

  if (!hasPreview(entityType)) {
    return trigger
  }

  return (
    <HoverCard openDelay={150}>
      <HoverCardTrigger asChild>{trigger}</HoverCardTrigger>
      <HoverCardContent className="w-80 p-3" align="start">
        <PreviewCard entityType={entityType} entityId={entityId} />
      </HoverCardContent>
    </HoverCard>
  )
}

function hasPreview(entityType: string): boolean {
  return entityType === 'customer' || entityType === 'subscription'
}

function defaultLabel(entityType: string, entityId: string): string {
  switch (entityType) {
    case 'customer':
      return 'Customer'
    case 'subscription':
      return 'Subscription'
    case 'invoice':
      return 'Invoice'
    case 'credit_note':
      return 'Credit note'
    case 'quote':
      return 'Quote'
    case 'plan':
      return 'Plan'
    case 'product':
      return 'Product'
    case 'add_on':
      return 'Add-on'
    case 'coupon':
      return 'Coupon'
    case 'billable_metric':
      return 'Billable metric'
    default:
      return entityId
  }
}

const PreviewCard = ({ entityType, entityId }: { entityType: string; entityId: string }) => {
  if (entityType === 'customer') return <CustomerPreview customerId={entityId} />
  if (entityType === 'subscription') return <SubscriptionPreview subscriptionId={entityId} />
  return null
}

const CustomerPreview = ({ customerId }: { customerId: string }) => {
  const query = useQuery(getCustomerById, { id: customerId }, { staleTime: 60_000 })
  if (query.isLoading) return <PreviewSkeleton />
  const c = query.data?.customer
  if (!c) return <p className="text-xs text-muted-foreground">Customer not available</p>
  return (
    <div className="space-y-1">
      <div className="text-sm font-semibold">{c.name}</div>
      {c.billingEmail && (
        <div className="text-xs text-muted-foreground">{c.billingEmail}</div>
      )}
      {c.currency && (
        <div className="text-[11px] text-muted-foreground">{c.currency}</div>
      )}
    </div>
  )
}

const SubscriptionPreview = ({ subscriptionId }: { subscriptionId: string }) => {
  const query = useQuery(
    getSubscriptionDetails,
    { subscriptionId },
    { staleTime: 60_000 },
  )
  if (query.isLoading) return <PreviewSkeleton />
  const s = query.data?.subscription
  if (!s) return <p className="text-xs text-muted-foreground">Subscription not available</p>
  return (
    <div className="space-y-1">
      <div className="text-sm font-semibold">{s.planName || 'Subscription'}</div>
      {s.customerName && (
        <div className="text-xs text-muted-foreground">{s.customerName}</div>
      )}
      {s.currency && (
        <div className="text-[11px] text-muted-foreground">{s.currency}</div>
      )}
    </div>
  )
}

const PreviewSkeleton = () => (
  <div className="space-y-2">
    <Skeleton height={14} width={140} />
    <Skeleton height={10} width={180} />
  </div>
)
