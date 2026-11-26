import { createQueryOptions, skipToken, useTransport } from '@connectrpc/connect-query'
import { Label, Skeleton } from '@md/ui'
import { useQueries } from '@tanstack/react-query'

import {
  type Line,
  type Owner,
  type Source,
  addOnOutcome,
  clausesOf,
  conditionLabel,
  levelKeyOf,
  levelSubject,
  resolvedEntitlementValue,
  rowPhrase,
  rowsLines,
} from '@/features/entitlements/model'
import { useResourceTypes } from '@/features/resources/hooks'
import { useQuery } from '@/lib/connectrpc'
import { listPlanVersionAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import {
  getResolvedEntitlementsForPlanVersion,
  listEntitlementsByEntity,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { type Entitlement, type Feature } from '@/rpc/api/entitlements/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

type Row = Pick<Entitlement, 'effect' | 'value' | 'condition' | 'level' | 'levelTypeId'>

const MAX_PLANS = 5

export const hasConsequence = (source: Source) => source === 'plan' || source === 'default' || source === 'addOn'

/** What the drawer's row yields where it lands: on its plan, as a default, or over each plan offering the add-on. */
export const RowConsequence = ({
  feature,
  source,
  owner,
  ownerName,
  existingId,
  row,
  multiInstance,
}: {
  feature: Feature
  source: Source
  owner: Owner
  ownerName?: string
  existingId?: string
  /** Unset while the form's value is invalid. */
  row: Row | undefined
  multiInstance: boolean
}) =>
  source === 'addOn' && owner.case === 'addOnId' ? (
    <AddOnConsequence feature={feature} addOnId={owner.value} row={row} multiInstance={multiInstance} />
  ) : (
    <OwnerConsequence
      feature={feature}
      source={source}
      owner={owner}
      ownerName={ownerName}
      existingId={existingId}
      row={row}
    />
  )

const OwnerConsequence = ({
  feature,
  source,
  owner,
  ownerName,
  existingId,
  row,
}: {
  feature: Feature
  source: Source
  owner: Owner
  ownerName?: string
  existingId?: string
  row: Row | undefined
}) => {
  const { byId: types } = useResourceTypes()
  const listed = useQuery(listEntitlementsByEntity, source === 'default' ? skipToken : { entity: { EntityId: owner } })
  const siblings = (source === 'default' ? feature.defaults : (listed.data?.entitlements ?? [])).filter(
    e => e.featureId === feature.id && !e.resourceId && !e.revokedAt && e.id !== existingId
  )
  if (source !== 'default' && listed.isLoading) return <Skeleton className="h-9 w-full" />
  const rows: Row[] = row ? [...siblings, row] : siblings
  return <Lines lines={rowsLines(rows, feature, types, ownerName)} />
}

const AddOnConsequence = ({
  feature,
  addOnId,
  row,
  multiInstance,
}: {
  feature: Feature
  addOnId: string
  row: Row | undefined
  multiInstance: boolean
}) => {
  const transport = useTransport()
  const { byId: types } = useResourceTypes()
  const plansQuery = useQuery(listPlans, { pagination: { page: 0, perPage: 100 } })
  const live = (plansQuery.data?.plans ?? []).filter(p => p.activeVersion)
  const offers = useQueries({
    queries: live.map(p =>
      createQueryOptions(listPlanVersionAddOns, { planVersionId: p.activeVersion!.id }, { transport })
    ),
  })
  const offering = live.filter((_, i) => offers[i]?.data?.planVersionAddOns.some(link => link.addOnId === addOnId))
  const shown = offering.slice(0, MAX_PLANS)
  // The plan's own-level value: the base an add-on row folds over.
  const ownLevel = !!row && levelKeyOf(row) === 'feature'
  const bases = useQueries({
    queries: (ownLevel ? shown : []).map(p =>
      createQueryOptions(getResolvedEntitlementsForPlanVersion, { planVersionId: p.activeVersion!.id }, { transport })
    ),
  })

  if (!row) return null
  if (plansQuery.isPending || offers.some(q => q.isPending) || bases.some(q => q.isPending))
    return <Skeleton className="h-9 w-full" />

  const clauses = clausesOf(row.condition)
  const when = Object.keys(clauses).length ? ` · ${conditionLabel(clauses, types)}` : ''
  const subject = levelSubject(levelKeyOf(row), feature, types)
  const lines: Line[] = shown.length
    ? shown.map((p, i) => ({
        label: `${subject} on ${p.name}${when}`,
        value: ownLevel
          ? addOnOutcome(
              resolvedEntitlementValue(bases[i]?.data?.entitlements.find(e => e.feature?.id === feature.id)),
              row,
              feature,
              multiInstance
            )
          : rowPhrase(row, feature),
      }))
    : [{ label: `${subject}${when}`, value: rowPhrase(row, feature) }]
  if (offering.length > shown.length)
    lines.push({ label: `${offering.length - shown.length} more plans`, value: '' })
  return <Lines lines={lines} />
}

const Lines = ({ lines }: { lines: Line[] }) =>
  lines.length ? (
    <div className="space-y-2">
      <Label className="text-xs font-normal dark:text-muted-foreground">Preview</Label>
      <div className="divide-y divide-border rounded-lg border border-border text-[13px]">
        {lines.map((l, i) => (
          <div key={`${l.label}-${i}`} className="flex items-start justify-between gap-3 px-3 py-2">
            <span className="min-w-0 text-muted-foreground">{l.label}</span>
            <span className="shrink-0 text-right font-medium tabular-nums">{l.value}</span>
          </div>
        ))}
      </div>
    </div>
  ) : null
