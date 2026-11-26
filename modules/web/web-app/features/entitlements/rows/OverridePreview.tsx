import { createQueryOptions, skipToken, useTransport } from '@connectrpc/connect-query'
import { Label, Skeleton, cn } from '@md/ui'
import { useQueries } from '@tanstack/react-query'

import { type Target } from '@/features/entitlements/explain/checkRequest'
import { useExplain } from '@/features/entitlements/explain/useExplain'
import { clausesOf, resolvedEntitlementValue, valueRank } from '@/features/entitlements/model'
import { plainValue } from '@/features/entitlements/rows/grantDisplay'
import {
  type OverrideColumn,
  type OverrideRow,
  overrideOutcomes,
  planNeighbours,
} from '@/features/entitlements/rows/overrideOutcomes'
import { LADDER_PAGE_SIZE } from '@/features/plans/ladder/useAddToPricingPage'
import { useResourceSubscriptions } from '@/features/resources/hooks'
import { useQuery } from '@/lib/connectrpc'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { type Feature, Tier } from '@/rpc/api/entitlements/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

/** What the customer (or its resource) gets without and with the override, now and after an upgrade or a downgrade. */
export function OverridePreview({
  customerId,
  resource,
  feature,
  typeId,
  existingId,
  row,
}: {
  customerId: string
  resource?: { id: string; type: ResourceType; selector: Record<string, string> }
  feature: Feature
  /** The level the override is on: unset for the customer. */
  typeId?: string
  existingId?: string
  /** Unset while the form's value is invalid. */
  row: OverrideRow | undefined
}) {
  const transport = useTransport()
  const target: Target = resource
    ? { kind: 'adhoc', type: resource.type, keyValue: resource.selector[resource.type.keys[0]] ?? '', attributes: {} }
    : { kind: 'customer' }
  const check = useExplain(customerId, feature.code, target)
  const level = check.data?.levels.find(l => (l.resourceTypeId ?? undefined) === typeId)
  const ownLevel = typeId === (feature.levelResourceTypeId || undefined)

  const subscriptions = useResourceSubscriptions(customerId)
  const deciding = level?.decidingSubscriptionId ? subscriptions.byId.get(level.decidingSubscriptionId) : undefined
  // A plan's resolved value is its own level's: other levels show now only.
  const plansQuery = useQuery(
    listPlans,
    deciding && ownLevel ? { pagination: { page: 0, perPage: LADDER_PAGE_SIZE } } : skipToken
  )
  const { upgrade, downgrade }: ReturnType<typeof planNeighbours> =
    deciding && plansQuery.data ? planNeighbours(plansQuery.data.plans, deciding.planId) : {}
  const neighbours = [upgrade, downgrade].flatMap(p => (p ? [p] : []))
  const resolved = useQueries({
    queries: neighbours.map(p =>
      createQueryOptions(getResolvedEntitlementsForPlanVersion, { planVersionId: p.activeVersion!.id }, { transport })
    ),
  })

  if (check.isLoading || plansQuery.isLoading || resolved.some(q => q.isPending))
    return <Skeleton className="h-24 w-full" />
  // A condition reads attributes the preview doesn't have.
  if (!level || !row?.value || Object.keys(clausesOf(row.condition)).length > 0) return null

  const columns = overrideOutcomes({
    feature,
    ownLevel,
    contributions: level.explain?.contributions ?? [],
    serverValue: level.value,
    existingId,
    row,
    tier: resource ? Tier.RESOURCE_OVERRIDE : Tier.CUSTOMER,
    plans: neighbours.map((p, i) => ({
      label: `On ${p.name}`,
      value: resolvedEntitlementValue(resolved[i]?.data?.entitlements.find(e => e.feature?.id === feature.id)),
    })),
  })
  if (!columns) return null

  return (
    <div className="space-y-2">
      <Label className="text-xs font-normal dark:text-muted-foreground">Preview</Label>
      <div className="overflow-hidden rounded-lg border border-border">
        <table className="w-full text-[13px]">
          <thead>
            <tr className="border-b border-border bg-muted/40 text-left text-xs text-muted-foreground">
              <th className="px-3 py-2 font-medium">
                <span className="sr-only">Value</span>
              </th>
              {columns.map(c => (
                <th key={c.label} className="px-3 py-2 font-medium">
                  {c.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            <tr className="border-b border-border">
              <td className="px-3 py-2 text-muted-foreground">Without this override</td>
              {columns.map(c => (
                <td key={c.label} className="px-3 py-2 tabular-nums">
                  {display(c.without, feature)}
                </td>
              ))}
            </tr>
            <tr>
              <td className="px-3 py-2 font-medium">With this override</td>
              {columns.map((c, i) => (
                <td
                  key={c.label}
                  className={cn('px-3 py-2 font-medium tabular-nums', i > 0 && isLoss(c, feature) && 'text-warning')}
                >
                  {display(c.with, feature)}
                  {c.addOnsDropped > 0 && (
                    <div className="text-xs font-normal text-muted-foreground">
                      {c.addOnsDropped === 1 ? 'add-on not counted' : `${c.addOnsDropped} add-ons not counted`}
                    </div>
                  )}
                </td>
              ))}
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  )
}

const display = (v: OverrideColumn['with'], f: Feature) => (v ? plainValue(v, f) : '—')

/** On another plan, the override keeps the customer below what that plan gives. */
const isLoss = (c: OverrideColumn, f: Feature) => {
  const without = valueRank(c.without, f)
  const withIt = valueRank(c.with, f)
  return without !== undefined && withIt !== undefined && withIt < without
}
