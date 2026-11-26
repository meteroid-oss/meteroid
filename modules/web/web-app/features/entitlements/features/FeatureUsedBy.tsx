import { Badge, Skeleton } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { type ReactNode, useMemo } from 'react'
import { Link } from 'react-router'

import { LocalId } from '@/components/LocalId'
import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { SimpleTable } from '@/components/table/SimpleTable'
import { grantValueLabel } from '@/features/entitlements/features/featureKind'
import {
  clausesOf,
  conditionLabel,
  levelKeyOf,
  levelLabel,
  type Owner,
} from '@/features/entitlements/model'
import { useResourceTypes } from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { getAddOn } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCustomersByIds } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listEntitlementsByFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  getPlanOverview,
  getPlanWithVersionByVersionId,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import type { Entitlement, Feature } from '@/rpc/api/entitlements/v1/models_pb'

type Group = 'plan' | 'addOn' | 'subscription' | 'quote' | 'override'

type Grant = { entitlement: Entitlement; owner: Owner; group: Group }

const GROUP_ORDER: Group[] = ['plan', 'addOn', 'subscription', 'quote', 'override']

const GROUP_NOUN: Record<Group, [string, string]> = {
  plan: ['plan', 'plans'],
  addOn: ['add-on', 'add-ons'],
  subscription: ['subscription', 'subscriptions'],
  quote: ['quote', 'quotes'],
  override: ['override', 'overrides'],
}

const groupOf = (owner: Owner): Group | undefined => {
  switch (owner.case) {
    case 'planId':
    case 'planVersionId':
      return 'plan'
    case 'addOnId':
      return 'addOn'
    case 'subscriptionId':
      return 'subscription'
    case 'quoteId':
      return 'quote'
    case 'customerId':
      return 'override'
    case 'featureId':
      return undefined
  }
}

const linkCls = 'text-sm font-medium text-brand hover:underline'

const SourceCell = ({ name, to, meta }: { name?: string; to?: string; meta: ReactNode }) => (
  <div className="flex min-w-0 flex-col">
    {name === undefined ? (
      <Skeleton className="my-0.5 h-4 w-28" />
    ) : to ? (
      <Link to={to} className={linkCls}>
        {name}
      </Link>
    ) : (
      <span className="text-sm font-medium">{name}</span>
    )}
    {typeof meta === 'string' ? <span className="text-xs text-muted-foreground">{meta}</span> : meta}
  </div>
)

const PlanVersionSource = ({ id }: { id: string }) => {
  const basePath = useBasePath()
  const plan = useQuery(getPlanWithVersionByVersionId, { localId: id }).data?.plan
  const version = plan?.version?.version
  return (
    <SourceCell
      name={plan?.plan?.name}
      to={plan?.plan ? `${basePath}/plans/${plan.plan.localId}/${version}` : undefined}
      meta={version !== undefined ? `Plan · v${version}` : 'Plan'}
    />
  )
}

const PlanSource = ({ id }: { id: string }) => {
  const basePath = useBasePath()
  const plan = useQuery(getPlanOverview, { localId: id }).data?.planOverview
  return (
    <SourceCell
      name={plan?.name}
      to={plan ? `${basePath}/plans/${plan.localId}` : undefined}
      meta="Plan · All versions"
    />
  )
}

const AddOnSource = ({ id }: { id: string }) => {
  const basePath = useBasePath()
  const addOn = useQuery(getAddOn, { addOnId: id }).data?.addOn
  return <SourceCell name={addOn?.name} to={`${basePath}/addons/${id}`} meta="Add-on" />
}

export const FeatureUsedBy = ({ feature }: { feature: Feature }) => {
  const basePath = useBasePath()
  const { byId: types } = useResourceTypes()
  const query = useQuery(listEntitlementsByFeature, { featureId: feature.id })

  const grants = useMemo<Grant[]>(
    () =>
      (query.data?.entitlements ?? [])
        .flatMap((entitlement): Grant[] => {
          const owner = entitlement.entity?.EntityId
          if (!owner || owner.case === undefined) return []
          const group = groupOf(owner)
          return group ? [{ entitlement, owner, group }] : []
        })
        .sort((a, b) => GROUP_ORDER.indexOf(a.group) - GROUP_ORDER.indexOf(b.group)),
    [query.data]
  )

  const customerIds = useMemo(
    () => [...new Set(grants.flatMap(g => (g.owner.case === 'customerId' ? [g.owner.value] : [])))],
    [grants]
  )
  const customersQuery = useQuery(listCustomersByIds, { ids: customerIds }, { enabled: customerIds.length > 0 })
  const customerNames = useMemo(
    () => new Map((customersQuery.data?.customers ?? []).map(c => [c.id, c.name])),
    [customersQuery.data]
  )

  const counts = useMemo(() => {
    const owners = new Map<Group, Set<string>>()
    grants.forEach(g => {
      const key = g.group === 'override' ? g.entitlement.id : g.owner.value
      owners.set(g.group, (owners.get(g.group) ?? new Set()).add(key))
    })
    return GROUP_ORDER.flatMap(group => {
      const n = owners.get(group)?.size ?? 0
      return n ? [`${n} ${GROUP_NOUN[group][n > 1 ? 1 : 0]}`] : []
    })
  }, [grants])

  const columns = useMemo<ColumnDef<Grant>[]>(
    () => [
      {
        id: 'source',
        header: 'Granted by',
        cell: ({ row }) => {
          const { owner, entitlement } = row.original
          switch (owner.case) {
            case 'planVersionId':
              return <PlanVersionSource id={owner.value} />
            case 'planId':
              return <PlanSource id={owner.value} />
            case 'addOnId':
              return <AddOnSource id={owner.value} />
            case 'subscriptionId':
              return (
                <SourceCell
                  name="Subscription"
                  to={`${basePath}/subscriptions/${owner.value}`}
                  meta={<LocalId localId={owner.value} className="max-w-24" buttonClassName="w-fit" />}
                />
              )
            case 'quoteId':
              return (
                <SourceCell
                  name="Quote"
                  to={`${basePath}/quotes/${owner.value}`}
                  meta={<LocalId localId={owner.value} className="max-w-24" buttonClassName="w-fit" />}
                />
              )
            case 'customerId':
              return (
                <SourceCell
                  name={customersQuery.isLoading ? undefined : (customerNames.get(owner.value) ?? owner.value)}
                  to={`${basePath}/customers/${owner.value}?tab=entitlements`}
                  meta={entitlement.resourceId ? 'Resource override' : 'Customer override'}
                />
              )
            default:
              return null
          }
        },
      },
      {
        id: 'value',
        header: 'Value',
        cell: ({ row }) => {
          const { entitlement, group } = row.original
          const level = levelKeyOf(entitlement)
          const clauses = clausesOf(entitlement.condition)
          const meta = [
            level === 'feature' ? undefined : levelLabel(level, feature, types),
            Object.keys(clauses).length ? `Only when ${conditionLabel(clauses, types)}` : undefined,
          ].filter(Boolean)
          return (
            <div className="flex min-w-0 flex-col">
              <span className="text-[13px]">
                {grantValueLabel(entitlement, feature, group === 'override')}
              </span>
              {meta.length > 0 && (
                <span className="text-xs text-muted-foreground">{meta.join(' · ')}</span>
              )}
            </div>
          )
        },
      },
    ],
    [basePath, customerNames, customersQuery.isLoading, feature, types]
  )

  if (query.isLoading)
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-24 w-full" />
      </div>
    )

  if (!grants.length) return <PanelEmptyState title="Not granted anywhere" quiet />

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-1.5">
        {counts.map(c => (
          <Badge key={c} variant="secondary" size="sm">
            {c}
          </Badge>
        ))}
      </div>
      <SimpleTable columns={columns} data={grants} />
    </div>
  )
}
