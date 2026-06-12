import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = {
  customer: Customer
}

export const ActivityCard = ({ customer }: Props) => {
  return (
    <EntityActivityTimeline
      entityType="customer"
      entityId={customer.id}
      emptyLabel="No activity yet for this customer"
    />
  )
}
