import { Badge } from '@md/ui'

import { SubscriptionSummary } from '@/rpc/portal/customer/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

interface CustomerPortalSubscriptionsProps {
  subscriptions: SubscriptionSummary[]
}

export const CustomerPortalSubscriptions = ({ subscriptions }: CustomerPortalSubscriptionsProps) => {
  if (subscriptions.length === 0) {
    return (
      <div className="text-center py-8 text-sm text-muted-foreground">
        No active subscriptions
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {subscriptions.map(subscription => (
        <div
          key={subscription.id}
          className="flex items-center justify-between p-4 border border-gray-200 rounded-lg hover:border-gray-300 transition-colors"
        >
          <div className="flex-1">
            <div className="font-medium text-sm text-gray-900">{subscription.planName}</div>
            <div className="text-xs text-gray-600 mt-1">
              {formatCurrency(Number(subscription.mrrCents), subscription.currency)} / month
            </div>
          </div>
          <Badge variant={subscription.status === 1 ? 'default' : 'secondary'} className="text-xs">
            {getSubscriptionStatusLabel(subscription.status)}
          </Badge>
        </div>
      ))}
    </div>
  )
}

const getSubscriptionStatusLabel = (status: number) => {
  const statusMap: Record<number, string> = {
    0: 'Pending',
    1: 'Active',
    2: 'Trial',
    3: 'Canceled',
    4: 'Ended',
  }
  return statusMap[status] || 'Unknown'
}