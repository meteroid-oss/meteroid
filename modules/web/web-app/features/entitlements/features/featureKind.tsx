import { Gauge, SlidersHorizontal, ToggleLeft } from 'lucide-react'

import { EFFECT, effectOf, kindLabel, valueLabel } from '@/features/entitlements/model'
import {
  type Entitlement,
  EntitlementEffect,
  type Feature,
  FeatureKind,
} from '@/rpc/api/entitlements/v1/models_pb'

import type { LucideIcon } from 'lucide-react'

export type KindKey = 'BOOLEAN' | 'VALUE' | 'LIMIT'

export const KIND_ICON: Record<KindKey, LucideIcon> = {
  BOOLEAN: ToggleLeft,
  VALUE: SlidersHorizontal,
  LIMIT: Gauge,
}

export const KIND_NAME: Record<KindKey, string> = {
  BOOLEAN: 'On/off',
  VALUE: 'Value',
  LIMIT: 'Limit',
}

export const kindKey = (kind: FeatureKind): KindKey =>
  kind === FeatureKind.LIMIT ? 'LIMIT' : kind === FeatureKind.VALUE ? 'VALUE' : 'BOOLEAN'

export const KindCell = ({ feature }: { feature: Feature }) => {
  const Icon = KIND_ICON[kindKey(feature.kind)]
  return (
    <span className="inline-flex items-center gap-1.5 text-sm">
      <Icon size={14} className="shrink-0 text-muted-foreground" />
      {kindLabel(feature)}
    </span>
  )
}

/** Plan rows show the value only; overrides spell out their Set to. */
export function grantValueLabel(
  e: Pick<Entitlement, 'effect' | 'value'>,
  f: Feature,
  isOverride: boolean
): string {
  const value = valueLabel(e.value, f)
  const effect = effectOf(e)
  switch (effect) {
    case EntitlementEffect.ADD:
      return `+${value}`
    case EntitlementEffect.RAISE_TO:
    case EntitlementEffect.CAP_AT:
      return `${EFFECT[effect].label} ${value}`
    default:
      return isOverride ? `${EFFECT[effect].label} ${value}` : value
  }
}

export const capitalize = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s)
