import { Badge, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { Fragment, type ReactNode } from 'react'

import {
  EFFECT,
  type Source,
  clausesOf,
  effectOf,
  isAppCounted,
  kindLabel,
  levelKeyOf,
  levelLabel,
  policyLabel,
  valueLabel,
} from '@/features/entitlements/model'
import { attributeKeyLabel, useResourceTypes } from '@/features/resources/hooks'
import {
  EntitlementEffect,
  type Entitlement,
  type EntitlementValue,
  type Feature,
  PolicyAtLimit,
} from '@/rpc/api/entitlements/v1/models_pb'
import { formatDateOnly } from '@/utils/date'

export const isOverrideSource = (source: Source) => source === 'customer' || source === 'resource'

const isWord = (v: EntitlementValue | undefined) => {
  const inner = v?.value
  return (
    inner?.case === 'booleanValue' ||
    (inner?.case === 'meteredValue' && (!inner.value.enabled || !inner.value.limit))
  )
}

const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1)

/** "On", "Unlimited", "10,000 requests": a value without its effect. */
export const plainValue = (v: EntitlementValue | undefined, f: Feature | undefined) =>
  isWord(v) ? capitalize(valueLabel(v, f)) : valueLabel(v, f)

/** "+300", "Raise to 2,000", "Set to 10", or the value alone on a plan. */
export function grantValueText(
  row: Pick<Entitlement, 'effect' | 'value'>,
  f: Feature | undefined,
  source: Source
): string {
  const value = plainValue(row.value, f)
  const effect = effectOf(row)
  switch (effect) {
    case EntitlementEffect.ADD:
      return isWord(row.value) ? `${EFFECT[effect].label} · ${value}` : `+${value}`
    case EntitlementEffect.RAISE_TO:
    case EntitlementEffect.CAP_AT:
      return `${EFFECT[effect].label} ${value}`
    default:
      return isOverrideSource(source) ? `${EFFECT[EntitlementEffect.SET].label} ${value}` : value
  }
}

const Dot = () => <span aria-hidden>·</span>

/** The value over its kind, level and policy notes. */
export const GrantValue = ({
  row,
  feature,
  source,
  metricLabel,
}: {
  row: Entitlement
  feature: Feature | undefined
  source: Source
  metricLabel?: string
}) => {
  const { byId } = useResourceTypes()
  const level = feature ? levelLabel(levelKeyOf(row), feature, byId) : undefined
  const policy = row.policyOverride ?? feature?.policy
  const meta: ReactNode[] = [
    feature && kindLabel(feature),
    level && level !== 'Customer' && level.toLowerCase(),
    // policyLabel leads with the at-limit part: "Overage allowed".
    policy?.atLimit === PolicyAtLimit.ALLOW_AND_BILL && policyLabel(policy, isAppCounted(feature)).split(' · ')[0],
    row.policyOverride && (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="cursor-default underline decoration-dotted underline-offset-2">custom policy</span>
        </TooltipTrigger>
        <TooltipContent>{capitalize(policyLabel(row.policyOverride, isAppCounted(feature)))}</TooltipContent>
      </Tooltip>
    ),
  ].filter(Boolean)

  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-sm font-medium">{grantValueText(row, feature, source)}</span>
      {(meta.length > 0 || metricLabel) && (
        <div className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
          {meta.map((m, i) => (
            <Fragment key={i}>
              {i > 0 && <Dot />}
              {m}
            </Fragment>
          ))}
          {metricLabel && (
            <Badge variant="outline" size="sm" className="ml-1 font-normal">
              {metricLabel}
            </Badge>
          )}
        </div>
      )}
    </div>
  )
}

/** Condition chips and the validity window, or "Always". */
export const GrantWhen = ({ row, badges }: { row: Entitlement; badges?: ReactNode }) => {
  const { byId } = useResourceTypes()
  const clauses = Object.entries(clausesOf(row.condition))
  const window = [
    row.validFrom && `From ${formatDateOnly(row.validFrom)}`,
    row.validUntil && `Until ${formatDateOnly(row.validUntil)}`,
  ]
    .filter(Boolean)
    .join(' · ')

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {badges}
      {clauses.map(([key, values]) => (
        <Badge key={key} variant="outline" size="sm" className="font-normal">
          {attributeKeyLabel(key, byId)}: {values.join(', ')}
        </Badge>
      ))}
      {window ? (
        <span className="text-xs text-muted-foreground">{window}</span>
      ) : (
        clauses.length === 0 && <span className="text-sm text-muted-foreground">Always</span>
      )}
    </div>
  )
}
