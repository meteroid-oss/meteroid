import { type FoldRow, foldRows, sameValue } from '@/features/entitlements/model'
import { ladderOf } from '@/features/plans/ladder/ladderOrder'
import {
  type Condition,
  type Contribution,
  DiscardReason,
  EntitlementEffect,
  type EntitlementValue,
  type Feature,
  Tier,
} from '@/rpc/api/entitlements/v1/models_pb'
import { type PlanOverview, PlanStatus } from '@/rpc/api/plans/v1/models_pb'

export type OverrideRow = { effect: EntitlementEffect; value?: EntitlementValue; condition?: Condition }

export type OverrideColumn = {
  label: string
  without: EntitlementValue | undefined
  with: EntitlementValue | undefined
  /** Add-on rows counted without the override and not with it. */
  addOnsDropped: number
}

/** Discards the fold doesn't decide: they hold whatever the other rows are. */
const SKIPPED_BEFORE_FOLD = new Set([
  DiscardReason.NOT_IN_EFFECT,
  DiscardReason.CONDITION_NOT_MET,
  DiscardReason.EFFECT_NOT_ALLOWED,
])

const NEW_ROW = 'new'

export const foldRowsOf = (contributions: Contribution[]): FoldRow[] =>
  contributions
    .filter(c => !SKIPPED_BEFORE_FOLD.has(c.discarded))
    .map(c => ({
      key: c.entitlementId,
      tier: c.tier,
      effect: c.effect,
      value: c.value,
      condition: c.condition,
      quantity: c.quantity,
      units: c.units,
    }))

/**
 * The level's value without and with an override, now and on other plans. Now refolds the
 * check's trace (an edited override's current version left out); another plan is its resolved
 * value under the customer's overrides. Unset when the refold doesn't give the server's value.
 */
export function overrideOutcomes({
  feature,
  ownLevel,
  contributions,
  serverValue,
  existingId,
  row,
  tier,
  plans,
}: {
  feature: Feature
  ownLevel: boolean
  contributions: Contribution[]
  serverValue: EntitlementValue | undefined
  existingId?: string
  row: OverrideRow
  tier: Tier
  plans: { label: string; value: EntitlementValue | undefined }[]
}): OverrideColumn[] | undefined {
  const all = foldRowsOf(contributions)
  if (!sameValue(foldRows(all, feature, ownLevel).value, serverValue, feature)) return undefined

  const others = all.filter(r => r.key !== existingId)
  const mine: FoldRow = { key: NEW_ROW, tier, effect: row.effect, value: row.value, condition: row.condition, quantity: 1 }
  const column = (label: string, rows: FoldRow[]): OverrideColumn => {
    const without = foldRows(rows, feature, ownLevel)
    const withIt = foldRows([...rows, mine], feature, ownLevel)
    const addOns = new Set(rows.filter(r => r.tier === Tier.ADD_ON).map(r => r.key))
    return {
      label,
      without: without.value,
      with: withIt.value,
      addOnsDropped: without.applied.filter(k => addOns.has(k) && !withIt.applied.includes(k)).length,
    }
  }

  const overrides = others.filter(r => r.tier === Tier.CUSTOMER || r.tier === Tier.RESOURCE_OVERRIDE)
  return [
    column('Now', others),
    ...plans.map(p =>
      column(p.label, [
        ...(p.value
          ? [{ key: 'plan', tier: Tier.PLAN_VERSION, effect: EntitlementEffect.SET, value: p.value, quantity: 1 }]
          : []),
        ...overrides,
      ])
    ),
  ]
}

const sellable = (p: PlanOverview) => p.planStatus === PlanStatus.ACTIVE && !!p.activeVersion

/** The nearest sellable plans above and below on the plan's self-serve ladder; customers upgrade up the ranks. */
export function planNeighbours(
  plans: PlanOverview[],
  planId: string
): { upgrade?: PlanOverview; downgrade?: PlanOverview } {
  const current = plans.find(p => p.id === planId)
  if (!current) return {}
  const ladder = ladderOf(plans.filter(p => p.productFamilyLocalId === current.productFamilyLocalId))
  const at = ladder.findIndex(p => p.id === planId)
  if (at < 0) return {}
  return {
    upgrade: ladder.slice(at + 1).find(sellable),
    downgrade: ladder.slice(0, at).reverse().find(sellable),
  }
}
