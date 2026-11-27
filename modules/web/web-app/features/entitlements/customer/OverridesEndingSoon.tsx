import { Button } from '@md/ui'
import { CalendarClock } from 'lucide-react'

import { overrideStatus } from '@/features/entitlements/customer/overrideStatus'
import { displayValue } from '@/features/entitlements/explain/display'
import { EFFECT, effectOf, type Owner } from '@/features/entitlements/model'
import { useQuery } from '@/lib/connectrpc'
import { listEntitlementsByEntity } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { parseUtc } from '@/utils/date'

/** The customer's overrides ending within a week, on its overview; nothing when none do. */
export const OverridesEndingSoon = ({
  customerId,
  onReview,
}: {
  customerId: string
  onReview: () => void
}) => {
  const owner: Owner = { case: 'customerId', value: customerId }
  const query = useQuery(listEntitlementsByEntity, { entity: { EntityId: owner } })
  const now = Date.now()
  const features = new Map((query.data?.features ?? []).map(f => [f.id, f]))
  const ending = (query.data?.entitlements ?? [])
    .filter(e => overrideStatus(e, now) === 'endsSoon')
    .sort((a, b) => parseUtc(a.validUntil!).getTime() - parseUtc(b.validUntil!).getTime())

  if (ending.length === 0) return null

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-warning/40 bg-warning/5 px-3 py-2 text-sm">
      <CalendarClock size={14} className="shrink-0 text-warning" aria-hidden />
      <span className="font-medium">
        {ending.length === 1
          ? 'An override ends this week'
          : `${ending.length} overrides end this week`}
      </span>
      <span className="min-w-0 flex-1 truncate text-muted-foreground">
        {ending
          .map(e => {
            const feature = features.get(e.featureId)
            const until = parseUtc(e.validUntil!).toLocaleDateString('en-US', {
              month: 'short',
              day: 'numeric',
            })
            return `${feature?.name ?? 'Feature'} (${EFFECT[effectOf(e)].label} ${displayValue(e.value, feature)}) on ${until}`
          })
          .join(' · ')}
      </span>
      <Button size="sm" variant="ghost" className="mt-0 h-7" onClick={onReview}>
        Review
      </Button>
    </div>
  )
}
