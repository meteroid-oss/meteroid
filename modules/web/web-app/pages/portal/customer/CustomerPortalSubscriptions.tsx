import { useSearchParams } from 'react-router-dom'

import { SubscriptionSummary } from '@/rpc/portal/customer/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

interface CustomerPortalSubscriptionsProps {
  subscriptions: SubscriptionSummary[]
}

const formatDate = (dateString: string): string => {
  const date = new Date(dateString)
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })
}

export const CustomerPortalSubscriptions = ({
  subscriptions,
}: CustomerPortalSubscriptionsProps) => {
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token')

  const handleSubscriptionClick = (subscriptionId: string) => {
    if (!token) return
    window.open(`/portal/subscription/${subscriptionId}?token=${token}`, '_blank')
  }
  if (subscriptions.length === 0) {
    return (
      <div className="text-center py-6">
        <p className="text-xs text-gray-500">No active subscriptions</p>
      </div>
    )
  }

  // Get the primary subscription (first one)
  const primarySubscription = subscriptions[0]

  return (
    <div>
      <div className="flex items-start justify-between mb-3">
        <div>
          <h3 className="text-base font-semibold text-gray-900 mb-1.5">
            {primarySubscription.planName}
          </h3>
          <div className="flex items-baseline gap-1.5">
            <span className="text-2xl font-semibold text-gray-900">
              {formatCurrency(Number(primarySubscription.mrrCents), primarySubscription.currency)}
            </span>
            <span className="text-sm text-gray-500">/ month</span>
          </div>
        </div>
        <button
          onClick={() => handleSubscriptionClick(primarySubscription.id)}
          className="text-xs text-gray-600 hover:text-gray-900 font-medium"
        >
          Manage →
        </button>
      </div>

      {primarySubscription.nextBillingDate && (
        <span className="text-xs py-2 text-gray-500">
          Renews on {formatDate(primarySubscription.nextBillingDate)}
        </span>
      )}

      {/* Show additional subscriptions if any */}
      {subscriptions.length > 1 && (
        <div className="mt-4 pt-3 border-t border-gray-200">
          <p className="text-xs font-medium text-gray-500 mb-2">Additional subscriptions</p>
          <div className="space-y-1.5">
            {subscriptions.slice(1).map(subscription => (
              <div
                key={subscription.id}
                onClick={() => handleSubscriptionClick(subscription.id)}
                className="flex items-center justify-between p-2.5 border border-gray-200 rounded hover:border-gray-300 cursor-pointer transition-colors"
              >
                <div>
                  <div className="text-sm font-medium text-gray-900">{subscription.planName}</div>
                  <div className="text-xs text-gray-500 mt-0.5">
                    {formatCurrency(Number(subscription.mrrCents), subscription.currency)} / month
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
