import { usePortalNav } from '../context'
import { Mono } from '../primitives'

// Two subscriptions on one plan are told apart by the usage each one bills: `team = mobile`.
export const UsageFilterTag = ({ subscriptionId }: { subscriptionId?: string }) => {
  const { overview } = usePortalNav()
  const filter = subscriptionId
    ? overview.activeSubscriptions.find(sub => sub.id === subscriptionId)?.usageFilter
    : undefined
  if (!filter) return null
  return (
    <Mono style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>
      {filter.property} = {filter.value}
    </Mono>
  )
}
