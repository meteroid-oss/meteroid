import Decimal from 'decimal.js'

import { DISCARD_LABEL, TIER_LABEL, valueLabel } from '@/features/entitlements/model'
import { formatDecimalGrouped } from '@/features/entitlements/utils'
import { singular } from '@/features/plans/charges/summary'
import {
  type Contribution,
  DiscardReason,
  type EntitlementValue,
  type Feature,
  Tier,
} from '@/rpc/api/entitlements/v1/models_pb'

const CAPITALIZED: Record<string, string> = { on: 'On', off: 'Off', unlimited: 'Unlimited' }

export const displayValue = (v: EntitlementValue | undefined, f?: Feature) => {
  const label = valueLabel(v, f)
  return CAPITALIZED[label] ?? label
}

export const isOffValue = (v: EntitlementValue | undefined) => {
  const inner = v?.value
  if (!inner?.case) return true
  if (inner.case === 'booleanValue' || inner.case === 'meteredValue') return !inner.value.enabled
  return false
}

export const isOnValue = (v: EntitlementValue | undefined) =>
  v?.value.case === 'booleanValue' && v.value.value.enabled

export const NOT_APPLIED: Record<DiscardReason, string> = {
  ...DISCARD_LABEL,
  [DiscardReason.REPLACED_BY_HIGHER_TIER]: 'replaced by a higher-priority grant',
  [DiscardReason.LOST_WITHIN_TIER]: 'a better grant from the same source wins',
  [DiscardReason.NO_BASE_ON_LEVEL]: 'nothing granted to adjust',
  [DiscardReason.EFFECT_NOT_ALLOWED]: 'effect not allowed here',
  [DiscardReason.CONDITION_NOT_MET]: '“Only when” not met',
}

/** "Plan · Growth v3", "Add-on · Extra seats × 2", "Customer override"; the tier alone without a name. */
export const sourceLabel = (c: Pick<Contribution, 'tier' | 'sourceName' | 'quantity'>) => {
  const tier = TIER_LABEL[c.tier]
  const prefixed = c.sourceName && c.tier !== Tier.DEFAULT && c.tier !== Tier.CUSTOMER
  const named = prefixed ? `${tier} · ${c.sourceName}` : (c.sourceName ?? tier)
  return c.quantity > 1 ? `${named} × ${c.quantity}` : named
}

/** A limit counted per unit (R-133): "5 seats" and the scaled amount "5,000 requests". */
export const perUnitAmount = (
  c: Pick<Contribution, 'value' | 'units'>,
  f: Feature,
  quantityName: string | undefined
) => {
  const inner = c.value?.value
  if (c.units === undefined || inner?.case !== 'meteredValue' || !inner.value.enabled || !inner.value.limit)
    return undefined
  // A quantity's name is its plural unit ("Seats").
  const name = quantityName ?? 'Units'
  const units = `${c.units.toLocaleString()} ${c.units === 1 ? singular(name) : name.trim().toLowerCase()}`
  const scaled = formatDecimalGrouped(new Decimal(inner.value.limit).times(c.units).toString())
  return { units, scaled: f.unit ? `${scaled} ${f.unit}` : scaled }
}
