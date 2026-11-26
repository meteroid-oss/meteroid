import { Badge, cn } from '@md/ui'
import { AlertTriangleIcon, ListFilterIcon } from 'lucide-react'
import { type ReactNode } from 'react'

import { type Target } from '@/features/entitlements/explain/checkRequest'
import { NOT_APPLIED, displayValue, perUnitAmount } from '@/features/entitlements/explain/display'
import { subscriptionName } from '@/features/entitlements/explain/useExplain'
import {
  EFFECT,
  TIER_LABEL,
  clausesOf,
  conditionLabel,
  effectOf,
  isWarningReason,
  reasonLabel,
  valueLabel,
} from '@/features/entitlements/model'
import { formatDecimalGrouped } from '@/features/entitlements/utils'
import { attributeKeyLabel, lower, useResourceSubscriptions, useResourceTypes } from '@/features/resources/hooks'
import { useQuery } from '@/lib/connectrpc'
import { listEntitlementsByEntity } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  type AppCountExplain,
  CheckReason,
  type Contribution,
  DiscardReason,
  EntitlementEffect,
  type EntitlementLevelState,
  type Feature,
  Tier,
} from '@/rpc/api/entitlements/v1/models_pb'
import { listManagedQuantities } from '@/rpc/api/quantities/v1/quantities-ManagedQuantitiesService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

import type { CheckEntitlementResponse } from '@/rpc/api/entitlements/v1/entitlements_pb'

type Props = {
  data: CheckEntitlementResponse
  feature: Feature
  customerId: string
  customerName: string
  target: Target
}

/** How each checked level reached its value, row by row, from the check's `explain`. */
export const ComputationTrace = ({ data, feature, customerId, customerName, target }: Props) => {
  const { byId: types } = useResourceTypes()
  const subscriptions = useResourceSubscriptions(customerId)
  const overrides = useQuery(listEntitlementsByEntity, {
    entity: { EntityId: { case: 'customerId', value: customerId } },
  })
  const overrideRows = new Map((overrides.data?.entitlements ?? []).map(e => [e.id, e]))
  const hasPerUnit = data.levels.some(l => l.explain?.contributions.some(c => c.units !== undefined))
  const quantities = useQuery(listManagedQuantities, {}, { enabled: hasPerUnit })
  const perUnitName = quantities.data?.quantities.find(q => q.perUnitFeatureIds.includes(feature.id))?.name
  const ownType = feature.levelResourceTypeId ? types.get(feature.levelResourceTypeId) : undefined

  const levelTitle = (l: EntitlementLevelState) => {
    if (!l.resourceTypeId) return customerName
    const type = types.get(l.resourceTypeId)
    const key = l.path.split('=').slice(1).join('=')
    if (!key) return type ? `Per ${lower(type.name)}` : 'Per resource'
    const stored =
      target.kind === 'stored' && target.type.id === l.resourceTypeId && target.resource.keyValue === key
        ? target.resource.name || key
        : key
    return `${type?.name ?? 'Resource'} ${stored}`
  }

  const grant = (c: Contribution, l: EntitlementLevelState) => {
    const row = c.tier === Tier.CUSTOMER || c.tier === Tier.RESOURCE_OVERRIDE ? overrideRows.get(c.entitlementId) : undefined
    const name = c.sourceName ?? (c.tier === Tier.RESOURCE_OVERRIDE ? levelTitle(l) : TIER_LABEL[c.tier])
    const meta = row
      ? [row.validUntil ? `until ${parseAndFormatDate(row.validUntil)}` : undefined, row.reason && `“${row.reason}”`]
          .filter(Boolean)
          .join(' · ')
      : undefined
    return { name, meta }
  }

  const notices: ReactNode[] = []
  if (data.reason === CheckReason.RESOURCE_REQUIRED && ownType)
    notices.push(
      <>
        {reasonLabel(data.reason, ownType)} · pass{' '}
        <code className="font-mono">{`resource: { type: "${ownType.code}", key }`}</code>
      </>
    )
  if (data.hint === 'resource_omitted')
    notices.push(<>Answered without a {ownType ? lower(ownType.name) : 'resource'}</>)
  if (data.attributesIgnored.length)
    notices.push(<>Ignored: {data.attributesIgnored.map(a => attributeKeyLabel(a, types)).join(', ')}</>)

  return (
    <div className="flex flex-col gap-3">
      {notices.map((n, i) => (
        <div
          key={i}
          className="flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs"
        >
          <AlertTriangleIcon size={14} className="mt-px shrink-0 text-warning" />
          <span>{n}</span>
        </div>
      ))}
      {data.levels.length === 0 && (
        <p className="text-[13px] text-muted-foreground">{reasonLabel(data.reason, ownType) ?? 'Nothing granted'}</p>
      )}
      {data.levels.map(l => {
        const deciding = l.decidingSubscriptionId ? subscriptions.byId.get(l.decidingSubscriptionId) : undefined
        const denied = !l.allowed && l.reason !== undefined && l.reason !== CheckReason.UNSPECIFIED
        return (
          <section key={`${l.resourceTypeId ?? ''}:${l.path}`} className="flex flex-col gap-1.5">
            {(data.levels.length > 1 || deciding || denied) && (
              <div className="flex flex-wrap items-center gap-2">
                {data.levels.length > 1 && <h4 className="text-[13px] font-medium">{levelTitle(l)}</h4>}
                {deciding && (
                  <Badge variant="outline" size="sm" className="font-normal">
                    Subscription · {subscriptionName(deciding)}
                  </Badge>
                )}
                {denied && (
                  <Badge
                    variant={isWarningReason(l.reason) ? 'warning' : 'destructive'}
                    size="sm"
                  >
                    {reasonLabel(l.reason, ownType)}
                  </Badge>
                )}
              </div>
            )}
            <LevelTable
              level={l}
              feature={feature}
              perUnitName={perUnitName}
              grant={c => grant(c, l)}
              condition={clauses => conditionLabel(clauses, types)}
            />
          </section>
        )
      })}
      {data.levels.length > 1 && (
        <div className="flex justify-end px-3 text-sm font-semibold">= {displayValue(data.value, feature)}</div>
      )}
    </div>
  )
}

const LevelTable = ({
  level,
  feature,
  perUnitName,
  grant,
  condition,
}: {
  level: EntitlementLevelState
  feature: Feature
  perUnitName: string | undefined
  grant: (c: Contribution) => { name: string; meta?: string }
  condition: (clauses: Record<string, string[]>) => string
}) => {
  const contributions = level.explain?.contributions ?? []
  const appCounts = level.explain?.appCounts ?? []
  const used = level.used !== undefined ? `${formatDecimalGrouped(level.used)} used` : undefined
  return (
    <div className="overflow-x-auto rounded-lg border border-border bg-muted/40">
      <table className="w-full min-w-[560px] text-[13px]">
        <thead>
          <tr className="border-b border-border text-left text-xs text-muted-foreground">
            <th className="w-28 px-3 py-2 font-medium">Source</th>
            <th className="px-3 py-2 font-medium">Grant</th>
            <th className="w-24 px-3 py-2 font-medium">Effect</th>
            <th className="w-32 px-3 py-2 font-medium">Amount</th>
            <th className="w-28 px-3 py-2 text-right font-medium">Running</th>
          </tr>
        </thead>
        <tbody>
          {contributions.length === 0 && (
            <tr>
              <td colSpan={5} className="px-3 py-2.5 text-muted-foreground">
                Nothing granted
              </td>
            </tr>
          )}
          {contributions.map((c, i) => {
            const applied = c.discarded === DiscardReason.UNSPECIFIED
            const effect = effectOf(c)
            const clauses = Object.entries(clausesOf(c.condition))
            const { name, meta } = grant(c)
            const perUnit = perUnitAmount(c, feature, perUnitName)
            const amount = `${effect === EntitlementEffect.ADD ? '+' : ''}${valueLabel(c.value, feature)}${
              perUnit ? ` × ${perUnit.units}` : ''
            }${c.quantity > 1 ? ` × ${c.quantity}` : ''}`
            return (
              <tr
                key={`${c.entitlementId}-${i}`}
                className={cn('border-b border-border/60 align-top last:border-0', !applied && 'text-muted-foreground')}
              >
                <td className="px-3 py-2.5">{TIER_LABEL[c.tier]}</td>
                <td className="px-3 py-2.5">
                  <div className="flex min-w-0 flex-col gap-1">
                    <span className={cn('font-medium', !applied && 'font-normal')}>{name}</span>
                    {meta && <span className="text-xs text-muted-foreground">{meta}</span>}
                    {clauses.length > 0 && (
                      <div className="flex flex-wrap gap-1" title="Only when">
                        {clauses.map(([attribute, values]) => (
                          <Badge key={attribute} variant="outline" size="sm" className="gap-1 font-normal">
                            <ListFilterIcon />
                            {condition({ [attribute]: values })}
                          </Badge>
                        ))}
                      </div>
                    )}
                    {!applied && <span className="text-xs">Not applied: {NOT_APPLIED[c.discarded]}</span>}
                  </div>
                </td>
                <td className="px-3 py-2.5 text-xs font-medium">{EFFECT[effect].label}</td>
                <td className="px-3 py-2.5 tabular-nums">
                  {amount}
                  {perUnit && <div className="text-xs text-muted-foreground">= {perUnit.scaled}</div>}
                </td>
                <td className="px-3 py-2.5 text-right font-medium tabular-nums">
                  {applied ? displayValue(c.running, feature) : '—'}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 border-t border-border bg-card px-3 py-2">
        <div className="flex flex-col gap-0.5 text-xs text-muted-foreground">
          {used && <span className="tabular-nums">{used}</span>}
          {appCounts.map(c => (
            <span key={c.path} className="tabular-nums">
              {appCountLine(c)}
            </span>
          ))}
        </div>
        <span className="text-[13px] font-semibold tabular-nums">= {displayValue(level.value, feature)}</span>
      </div>
    </div>
  )
}

const appCountLine = (c: AppCountExplain) => {
  const count = Number(c.count) < 0 ? '0' : formatDecimalGrouped(c.count)
  const parts = [`Count ${count}`]
  if (c.lastSetValue)
    parts.push(`set to ${formatDecimalGrouped(c.lastSetValue)}${c.lastSetAt ? ` on ${parseAndFormatDate(c.lastSetAt)}` : ''}`)
  if (Number(c.addsSince) > 0) {
    const sum = Number(c.addsSum) >= 0 ? `+${formatDecimalGrouped(c.addsSum)}` : formatDecimalGrouped(c.addsSum)
    parts.push(`${sum} since${c.lastAddAt ? ` (last ${parseAndFormatDate(c.lastAddAt)})` : ''}`)
  }
  return parts.join(' · ')
}
