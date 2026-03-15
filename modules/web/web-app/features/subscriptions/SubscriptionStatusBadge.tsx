import { Badge } from '@md/ui'

import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'

interface Props {
  status: SubscriptionStatus
}

export const SubscriptionStatusBadge = ({ status }: Props) => {
  switch (status) {
    case SubscriptionStatus.ACTIVE:
      return <Badge variant="success">Active</Badge>
    case SubscriptionStatus.CANCELED:
      return <Badge variant="secondary">Canceled</Badge>
    case SubscriptionStatus.ENDED:
      return <Badge variant="secondary">Ended</Badge>
    case SubscriptionStatus.PENDING:
      return <Badge variant="warning">Pending</Badge>
    case SubscriptionStatus.TRIALING:
      return <Badge variant="outline">Trial</Badge>
    case SubscriptionStatus.TRIAL_EXPIRED:
      return <Badge variant="warning">Trial Expired</Badge>
    case SubscriptionStatus.ERRORED:
      return <Badge variant="destructive">Errored</Badge>
    default:
      return null
  }
}
