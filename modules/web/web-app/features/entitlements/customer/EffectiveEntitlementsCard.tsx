import {
  Badge,
  Card,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { ChevronRightIcon } from 'lucide-react'
import { Fragment, type KeyboardEvent, useState } from 'react'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { GroupHeaderRow } from '@/components/table/GroupHeaderRow'
import { ComputationTrace } from '@/features/entitlements/explain/ComputationTrace'
import { EvaluateFor } from '@/features/entitlements/explain/EvaluateFor'
import { type Target, isTargetReady, targetRequest } from '@/features/entitlements/explain/checkRequest'
import { displayValue, isOffValue, isOnValue, sourceLabel } from '@/features/entitlements/explain/display'
import { subscriptionName, useExplain } from '@/features/entitlements/explain/useExplain'
import { isAppCounted, isLimit, kindLabel, reasonLabel } from '@/features/entitlements/model'
import { productRows } from '@/features/entitlements/productGroups'
import { formatDecimalGrouped } from '@/features/entitlements/utils'
import { lower, useResourceSubscriptions, useResourceTypes } from '@/features/resources/hooks'
import { useQuery } from '@/lib/connectrpc'
import {
  getEntitlementSnapshot,
  listFeatures,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  CalendarUnit,
  CheckReason,
  type Feature,
  FeatureKind,
  type FeaturePolicy,
  FeatureStatus,
  PolicyResets,
  type SnapshotFeature,
  type SnapshotLevel,
} from '@/rpc/api/entitlements/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

type Props = {
  customerId: string
  customerName: string
  creditPaid: Set<string>
}

export const EffectiveEntitlementsCard = ({ customerId, customerName, creditPaid }: Props) => {
  const { types, byId: typesById } = useResourceTypes()
  const [target, setTarget] = useState<Target>({ kind: 'customer' })
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const evaluated: Target = isTargetReady(target) ? target : { kind: 'customer' }

  const snapshot = useQuery(getEntitlementSnapshot, { customerId, ...targetRequest(evaluated) })
  const featuresQuery = useQuery(listFeatures, {
    pagination: { page: 0, perPage: 500 },
    statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
  })
  const features = new Map((featuresQuery.data?.features ?? []).map(f => [f.id, f]))
  const { rows, headerAt } = productRows(
    [...(snapshot.data?.features ?? [])].sort((a, b) =>
      (a.feature?.name ?? '').localeCompare(b.feature?.name ?? '')
    ),
    sf => sf.feature?.product
  )

  const toggle = (id: string) =>
    setExpanded(prev => {
      const next = new Set(prev)
      if (!next.delete(id)) next.add(id)
      return next
    })

  return (
    <Card className="overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-4">
        <h3 className="text-sm font-semibold">Effective entitlements</h3>
        {types.length > 0 && (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[13px] text-muted-foreground">Evaluate for</span>
            <EvaluateFor customerId={customerId} customerName={customerName} value={target} onChange={setTarget} />
          </div>
        )}
      </div>
      {snapshot.isLoading || featuresQuery.isLoading ? (
        <div className="flex flex-col gap-2 px-5 pb-5">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      ) : snapshot.error ? (
        <PanelEmptyState tone="destructive" title={snapshot.error.rawMessage} />
      ) : rows.length === 0 ? (
        <PanelEmptyState quiet title="No entitlements" />
      ) : (
        <Table className="min-w-[560px]">
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-5">Feature</TableHead>
              <TableHead>Kind</TableHead>
              <TableHead>Value</TableHead>
              <TableHead>From</TableHead>
              <TableHead className="w-10 pr-5" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((sf, i) => {
              const id = sf.feature?.id ?? ''
              const header = headerAt(i)
              return (
                <Fragment key={id}>
                  {header && <GroupHeaderRow colSpan={5}>{header}</GroupHeaderRow>}
                  <FeatureRow
                    snapshot={sf}
                    feature={features.get(id)}
                    target={evaluated}
                    customerId={customerId}
                    customerName={customerName}
                    paidWithCredit={creditPaid.has(id)}
                    expanded={expanded.has(id)}
                    onToggle={() => toggle(id)}
                    types={typesById}
                  />
                </Fragment>
              )
            })}
          </TableBody>
        </Table>
      )}
    </Card>
  )
}

type RowProps = {
  snapshot: SnapshotFeature
  feature?: Feature
  target: Target
  customerId: string
  customerName: string
  paidWithCredit: boolean
  expanded: boolean
  onToggle: () => void
  types: Map<string, ResourceType>
}

const ownLevelOf = <L extends Pick<SnapshotLevel, 'resourceTypeId'>>(levels: L[], feature?: Feature) =>
  levels.find(l => (l.resourceTypeId ?? undefined) === (feature?.levelResourceTypeId ?? undefined)) ?? levels[0]

const FeatureRow = ({
  snapshot: sf,
  feature,
  target,
  customerId,
  customerName,
  paidWithCredit,
  expanded,
  onToggle,
  types,
}: RowProps) => {
  const trace = useExplain(customerId, sf.feature?.code, target, expanded && !!feature)
  const subscriptions = useResourceSubscriptions(customerId)
  const own = ownLevelOf(sf.levels, feature)

  const from = (() => {
    if (own?.origin) return sourceLabel(own.origin)
    const deciding = own?.decidingSubscriptionId ? subscriptions.byId.get(own.decidingSubscriptionId) : undefined
    return deciding ? `Subscription · ${subscriptionName(deciding)}` : undefined
  })()

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onToggle()
    }
  }

  return (
    <Fragment>
      <TableRow
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={onKeyDown}
        className={cn('cursor-pointer', expanded && 'border-b-0')}
      >
        <TableCell className="py-3 pl-5 align-top">
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-sm font-medium">{sf.feature?.name}</span>
            <span className="font-mono text-xs text-muted-foreground">{sf.feature?.code}</span>
          </div>
        </TableCell>
        <TableCell className="py-3 align-top">
          {feature && (
            <Badge variant="outline" size="sm">
              {kindLabel(feature)}
            </Badge>
          )}
        </TableCell>
        <TableCell className="py-3 align-top">
          <ValueCell snapshot={sf} own={own} feature={feature} types={types} paidWithCredit={paidWithCredit} />
        </TableCell>
        <TableCell className="py-3 align-top text-[13px] text-muted-foreground">{from ?? '—'}</TableCell>
        <TableCell className="py-3 pr-5 align-top">
          <ChevronRightIcon
            size={16}
            className={cn('text-muted-foreground transition-transform', expanded && 'rotate-90')}
          />
        </TableCell>
      </TableRow>
      {expanded && (
        <TableRow className="hover:bg-transparent">
          <TableCell colSpan={5} className="px-5 pb-4 pt-0">
            {!feature ? null : trace.isLoading ? (
              <Skeleton className="h-24 w-full" />
            ) : trace.error ? (
              <p className="text-[13px] text-destructive">{trace.error.rawMessage}</p>
            ) : (
              trace.data && (
                <ComputationTrace
                  data={trace.data}
                  feature={feature}
                  customerId={customerId}
                  customerName={customerName}
                  target={target}
                />
              )
            )}
          </TableCell>
        </TableRow>
      )}
    </Fragment>
  )
}

const hasReason = (l: Pick<SnapshotLevel, 'reason'> | undefined) =>
  l?.reason !== undefined && l.reason !== CheckReason.UNSPECIFIED

const levelName = (l: SnapshotLevel, feature: Feature | undefined, types: Map<string, ResourceType>) => {
  const type = l.resourceTypeId ? types.get(l.resourceTypeId) : undefined
  const own = feature?.levelResourceTypeId ? types.get(feature.levelResourceTypeId) : undefined
  return type ? `Per ${lower(type.name)}` : own ? `Across all ${lower(own.pluralName)}` : 'Customer'
}

const ValueCell = ({
  snapshot: sf,
  own,
  feature,
  types,
  paidWithCredit,
}: {
  snapshot: SnapshotFeature
  own: SnapshotLevel | undefined
  feature: Feature | undefined
  types: Map<string, ResourceType>
  paidWithCredit: boolean
}) => {
  const ownType = feature?.levelResourceTypeId ? types.get(feature.levelResourceTypeId) : undefined
  const others = sf.levels.filter(l => l !== own && (l.value || hasReason(l)))
  const usage = isLimit(feature) && own?.used !== undefined ? own : undefined

  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      {!sf.value && hasReason(own) ? (
        <Badge variant={own?.reason === CheckReason.RESOURCE_REQUIRED ? 'warning' : 'outline'} size="sm">
          {reasonLabel(own?.reason, ownType)}
        </Badge>
      ) : (
        <span
          className={cn(
            'text-sm font-medium tabular-nums',
            (!sf.value || isOffValue(sf.value)) && 'text-muted-foreground',
            isOnValue(sf.value) && 'text-success'
          )}
        >
          {sf.value ? displayValue(sf.value, feature) : feature?.kind === FeatureKind.VALUE ? '—' : 'Off'}
        </span>
      )}
      {usage && <UsageBar level={usage} appCounted={isAppCounted(feature)} />}
      {isLimit(feature) && own?.usageUnavailable && (
        <span className="text-xs text-muted-foreground">{reasonLabel(CheckReason.USAGE_UNAVAILABLE, ownType)}</span>
      )}
      {others.map(l => (
        <span key={`${l.resourceTypeId ?? ''}:${l.path}`} className="text-xs text-muted-foreground">
          {levelName(l, feature, types)}:{' '}
          {hasReason(l) && !l.value
            ? reasonLabel(l.reason, l.resourceTypeId ? types.get(l.resourceTypeId) : undefined)
            : `${l.used !== undefined ? `${formatDecimalGrouped(l.used)} / ` : ''}${displayValue(l.value, feature)}`}
        </span>
      ))}
      {paidWithCredit && (
        <Badge variant="outline" size="sm" className="font-normal">
          Paid with credit
        </Badge>
      )}
    </div>
  )
}

const UsageBar = ({ level, appCounted }: { level: SnapshotLevel; appCounted: boolean }) => {
  const metered = level.value?.value.case === 'meteredValue' ? level.value.value.value : undefined
  const limit = metered?.enabled ? metered.limit : undefined
  const used = Number(level.used ?? 0)
  const pct = limit && Number(limit) > 0 ? (used / Number(limit)) * 100 : limit ? 100 : undefined
  const resets = appCounted ? undefined : resetsLabel(level.policy)

  return (
    <div className="flex flex-col gap-1">
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        {pct !== undefined && (
          <div
            className={cn(
              'h-full rounded-full',
              pct >= 100 ? 'bg-destructive' : pct >= 80 ? 'bg-warning' : 'bg-success'
            )}
            style={{ width: `${Math.min(pct, 100)}%` }}
          />
        )}
      </div>
      <span className="text-xs text-muted-foreground tabular-nums">
        {limit
          ? `${formatDecimalGrouped(level.used ?? '0')} / ${formatDecimalGrouped(limit)}`
          : `${formatDecimalGrouped(level.used ?? '0')} used`}
        {resets &&
          (level.resetAt ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="underline decoration-dotted underline-offset-2"> · {resets}</span>
              </TooltipTrigger>
              <TooltipContent>{parseAndFormatDate(level.resetAt)}</TooltipContent>
            </Tooltip>
          ) : (
            ` · ${resets}`
          ))}
      </span>
    </div>
  )
}

const UNIT: Record<CalendarUnit, [adverb: string, noun: string]> = {
  [CalendarUnit.HOUR]: ['hourly', 'hour'],
  [CalendarUnit.DAY]: ['daily', 'day'],
  [CalendarUnit.WEEK]: ['weekly', 'week'],
  [CalendarUnit.MONTH]: ['monthly', 'month'],
  [CalendarUnit.YEAR]: ['yearly', 'year'],
}

const resetsLabel = (p: FeaturePolicy | undefined) => {
  if (!p) return undefined
  const [adverb, noun] = UNIT[p.period?.unit ?? CalendarUnit.MONTH]
  const n = p.period?.interval ?? 1
  const span = n === 1 ? noun : `${n} ${noun}s`
  switch (p.resets) {
    case PolicyResets.UNSPECIFIED:
    case PolicyResets.BILLING_CYCLE:
      return 'resets each billing cycle'
    case PolicyResets.CALENDAR:
    case PolicyResets.FIXED:
      return n === 1 ? `resets ${adverb}` : `resets every ${span}`
    case PolicyResets.SLIDING:
      return `over the last ${span}`
    case PolicyResets.NEVER:
      return undefined
  }
}
