import { Badge, Button, Card, Skeleton, cn } from '@md/ui'
import { useEffect, useState } from 'react'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { useMembers } from '@/features/accounts/hooks/useMembers'
import { overrideStatus } from '@/features/entitlements/customer/overrideStatus'
import { displayValue } from '@/features/entitlements/explain/display'
import {
  EFFECT,
  type Owner,
  clausesOf,
  conditionLabel,
  effectOf,
  levelKeyOf,
  levelLabel,
} from '@/features/entitlements/model'
import { productSections } from '@/features/entitlements/productGroups'
import { RowSheet } from '@/features/entitlements/rows/RowSheet'
import { useResourceTypes } from '@/features/resources/hooks'
import { usePermissions } from '@/hooks/usePermissions'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import { listEntitlementsByEntity } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { parseUtc } from '@/utils/date'

import type { Actor, Entitlement, Feature } from '@/rpc/api/entitlements/v1/models_pb'

type Editing = { existing?: Entitlement; feature?: Feature; temporary?: boolean } | undefined

/** The customer-wide overrides; resource overrides live on each resource. */
export const OverridesCard = ({ customerId }: { customerId: string }) => {
  const canEdit = usePermissions().has('catalog:manage')
  const owner: Owner = { case: 'customerId', value: customerId }
  const query = useQuery(listEntitlementsByEntity, { entity: { EntityId: owner } })
  const [editing, setEditing] = useState<Editing>()
  const [showEnded, setShowEnded] = useState(false)
  // `?override=temporary` opens a temporary override, as links from the customer's overview do.
  const [requested, setRequested] = useQueryState('override', '', { replace: true })
  useEffect(() => {
    if (requested !== 'temporary' || !canEdit) return
    setEditing({ temporary: true })
    setRequested('')
  }, [requested, canEdit, setRequested])

  const features = new Map((query.data?.features ?? []).map(f => [f.id, f]))
  const now = Date.now()
  const rows = (query.data?.entitlements ?? [])
    .filter(e => !e.resourceId)
    .sort((a, b) =>
      (features.get(a.featureId)?.name ?? '').localeCompare(features.get(b.featureId)?.name ?? '')
    )
  const ended = rows.filter(e => overrideStatus(e, now) === 'ended')
  const active = rows.filter(e => !ended.includes(e))

  const card = (e: Entitlement) => (
    <OverrideItem
      key={e.id}
      row={e}
      feature={features.get(e.featureId)}
      now={now}
      onOpen={
        canEdit ? () => setEditing({ existing: e, feature: features.get(e.featureId) }) : undefined
      }
    />
  )
  const list = (items: Entitlement[]) => {
    const sections = productSections(items, e => features.get(e.featureId)?.product)
    return sections ? (
      <div className="flex flex-col gap-4">
        {sections.map(section => (
          <div key={section.key} className="flex flex-col gap-2">
            <span className="text-xs font-medium text-muted-foreground">{section.label}</span>
            {section.items.map(card)}
          </div>
        ))}
      </div>
    ) : (
      <div className="flex flex-col gap-2">{items.map(card)}</div>
    )
  }

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">Overrides</h3>
        {canEdit && (
          <div className="flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              className="mt-0"
              title="Add an override that ends on a date"
              onClick={() => setEditing({ temporary: true })}
            >
              Temporary
            </Button>
            <Button size="sm" variant="outline" className="mt-0" onClick={() => setEditing({})}>
              Add override
            </Button>
          </div>
        )}
      </div>

      {query.isLoading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </div>
      ) : query.error ? (
        <PanelEmptyState tone="destructive" title={query.error.rawMessage} className="py-6" />
      ) : (
        <>
          {active.length === 0 ? (
            <PanelEmptyState quiet title="No overrides" className="py-6" />
          ) : (
            list(active)
          )}
          {ended.length > 0 && (
            <Button
              size="sm"
              variant="ghost"
              className="mt-0 self-start text-muted-foreground"
              onClick={() => setShowEnded(v => !v)}
            >
              {showEnded ? 'Hide ended' : `Show ended (${ended.length})`}
            </Button>
          )}
          {showEnded && list(ended)}
        </>
      )}

      {editing && <RowSheet owner={owner} {...editing} onClose={() => setEditing(undefined)} />}
    </Card>
  )
}

const OverrideItem = ({
  row,
  feature,
  now,
  onOpen,
}: {
  row: Entitlement
  feature?: Feature
  now: number
  onOpen?: () => void
}) => {
  const { byId: types } = useResourceTypes()
  const { byId: members } = useMembers()
  const clauses = clausesOf(row.condition)
  const level = levelKeyOf(row)
  const scope = [
    level !== 'feature' ? levelLabel(level, feature, types) : undefined,
    Object.keys(clauses).length ? `Only when ${conditionLabel(clauses, types)}` : undefined,
  ].filter(Boolean)
  const author = actorName(row.createdBy, id => members.get(id)?.displayName)
  const meta = [author, row.reason && `“${row.reason}”`].filter(Boolean).join(' · ')
  const status = overrideStatus(row, now)
  const ended = status === 'ended'

  return (
    <button
      type="button"
      disabled={!onOpen}
      onClick={onOpen}
      className={cn(
        'w-full space-y-1 rounded-md border border-border p-3 text-left transition-colors enabled:hover:bg-muted/40 disabled:cursor-default',
        ended && 'text-muted-foreground'
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <span className="min-w-0 truncate text-[13px] font-medium">
          {feature?.name ?? row.featureId}
        </span>
        <span className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
          {status === 'endsSoon' && (
            <Badge variant="warning" size="sm">
              Ends soon
            </Badge>
          )}
          {validity(row, now)}
        </span>
      </div>
      <div className="flex items-baseline gap-1.5 text-[13px]">
        <span className="text-xs font-medium">{EFFECT[effectOf(row)].label}</span>
        <span className="tabular-nums">{displayValue(row.value, feature)}</span>
      </div>
      {scope.length > 0 && <div className="text-xs text-muted-foreground">{scope.join(' · ')}</div>}
      {meta && <div className="truncate text-xs text-muted-foreground">{meta}</div>}
    </button>
  )
}

const shortDate = (value: string) => {
  const date = parseUtc(value)
  return date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
  })
}

const validity = (row: Entitlement, now: number) => {
  if (row.validFrom && parseUtc(row.validFrom).getTime() > now)
    return `Starts ${shortDate(row.validFrom)}`
  if (!row.validUntil) return 'No end'
  return parseUtc(row.validUntil).getTime() <= now
    ? `Ended ${shortDate(row.validUntil)}`
    : `Ends ${shortDate(row.validUntil)}`
}

const actorName = (actor: Actor | undefined, memberName: (id: string) => string | undefined) => {
  switch (actor?.type) {
    case 'USER':
      return (actor.id && memberName(actor.id)) || 'User'
    case 'API_KEY':
      return 'API key'
    case 'SYSTEM':
      return 'System'
    default:
      return undefined
  }
}
