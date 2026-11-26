import { createQueryOptions, useMutation, useTransport } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Tabs,
  TabsList,
  TabsTrigger,
  cn,
} from '@md/ui'
import { useQueries } from '@tanstack/react-query'
import { ArchiveIcon, Boxes, Columns3Icon, HelpCircleIcon, MoreVerticalIcon, PencilIcon, PlusIcon } from 'lucide-react'
import { Fragment, type KeyboardEvent, type ReactNode, useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { toast } from 'sonner'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { ListToolbarSearch } from '@/components/layouts/ListPageLayout'
import { GroupHeaderRow } from '@/components/table/GroupHeaderRow'
import { ExplainDialog } from '@/features/entitlements/explain/ExplainDialog'
import { type Target } from '@/features/entitlements/explain/checkRequest'
import { isLimit, reasonLabel, valueLabel } from '@/features/entitlements/model'
import { productGroups, productRows } from '@/features/entitlements/productGroups'
import { EntitlementRowsEditor } from '@/features/entitlements/rows/EntitlementRowsEditor'
import { formatDecimalGrouped } from '@/features/entitlements/utils'
import { ResourceDialog } from '@/features/resources/ResourceDialog'
import { AttributeBadges } from '@/features/resources/TypeDetail'
import {
  humanize,
  keyLabel,
  lower,
  resourceLabel,
  resourceSelector,
  useCustomerResources,
  useInvalidateResources,
  useResourceSubscriptions,
  useResourceTypes,
} from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import {
  getEntitlementSnapshot,
  getEntitlementSnapshots,
  listEntitlementsByEntity,
  listFeatures,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  CheckReason,
  type Feature,
  FeatureStatus,
  type SnapshotFeature,
  type SnapshotLevel,
} from '@/rpc/api/entitlements/v1/models_pb'
import { archiveResource } from '@/rpc/api/resources/v1/resources-ResourcesService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { Resource, ResourceType } from '@/rpc/api/resources/v1/models_pb'
import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'

type Props = { customerId: string; customerName: string; type?: ResourceType }
type Scope = { customerId: string; customerName: string; type: ResourceType }
type Coverage = { own: Subscription[]; customerWide: Subscription[] }

const MAX_USAGE_COLUMNS = 3
// The most resources one batched snapshot reads.
const SNAPSHOT_BATCH = 100

const MANAGE_RESOURCES = 'customers:manage'
const MANAGE_SUBSCRIPTIONS = 'billing:subscriptions:manage'
const MANAGE_ENTITLEMENTS = 'catalog:manage'

/** A customer's stored resources, of `type` or, without it, of the type picked in the toolbar. */
export const ResourcesTab = ({ customerId, customerName, type: fixed }: Props) => {
  const { types } = useResourceTypes()
  const [picked, setPicked] = useState<string>()
  const type = fixed ?? types.find(t => t.id === picked) ?? types[0]
  if (!type) return null
  const switcher =
    !fixed && types.length > 1 ? (
      <Tabs value={type.id} onValueChange={setPicked}>
        <TabsList variant="ghost">
          {types.map(t => (
            <TabsTrigger key={t.id} value={t.id}>
              {t.pluralName}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    ) : undefined
  return (
    <ResourcesCard key={type.id} customerId={customerId} customerName={customerName} type={type} switcher={switcher} />
  )
}

const ResourcesCard = ({ customerId, customerName, type, switcher }: Scope & { switcher?: ReactNode }) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const { has } = usePermissions()
  const canManage = has(MANAGE_RESOURCES)
  const canSubscribe = has(MANAGE_SUBSCRIPTIONS)
  const showConfirmModal = useConfirmationModal()
  const invalidate = useInvalidateResources()
  const { resources, isLoading } = useCustomerResources(customerId, type.id)
  const subscriptions = useResourceSubscriptions(customerId)
  const overrides =
    useQuery(listEntitlementsByEntity, { entity: { EntityId: { case: 'customerId', value: customerId } } }).data
      ?.entitlements ?? []
  const limits = (
    useQuery(listFeatures, {
      levelResourceTypeId: type.id,
      statuses: [FeatureStatus.ACTIVE],
      pagination: { page: 0, perPage: 100 },
    }).data?.features ?? []
  ).filter(isLimit)
  const [columns, setColumns] = useState<string[]>()
  const usage = limits.filter(f => (columns ?? limits.slice(0, MAX_USAGE_COLUMNS).map(l => l.id)).includes(f.id))
  const toggleColumn = (id: string) =>
    setColumns(
      usage.some(f => f.id === id) ? usage.filter(f => f.id !== id).map(f => f.id) : [...usage.map(f => f.id), id]
    )

  const [search, setSearch] = useState('')
  const [adding, setAdding] = useState(false)
  const [editing, setEditing] = useState<Resource>()
  const [openedId, setOpenedId] = useState<string>()
  const opened = openedId ? resources.find(r => r.id === openedId) : undefined
  const [explain, setExplain] = useState<Target>()
  const usageByResource = useResourceUsage(customerId, resources, usage)

  useEffect(() => {
    if (openedId && !isLoading && !resources.some(r => r.id === openedId)) setOpenedId(undefined)
  }, [openedId, isLoading, resources])

  const archive = useMutation(archiveResource, {
    onSuccess: () => {
      toast.success(`${type.name} archived`)
      return invalidate()
    },
    onError: err => toast.error(err.message),
  })

  const needle = search.trim().toLowerCase()
  const shown = resources.filter(
    r => !needle || r.keyValue.toLowerCase().includes(needle) || r.name?.toLowerCase().includes(needle)
  )
  const coverage = (r: Resource): Coverage => ({
    own: subscriptions.forResource(r.id),
    customerWide: subscriptions.forResource(undefined),
  })
  const overrideCount = (r: Resource) => overrides.filter(e => e.resourceId === r.id).length
  const confirmArchive = (r: Resource, onArchived?: () => void) =>
    showConfirmModal(
      () => archive.mutate({ id: r.id }, { onSuccess: onArchived }),
      { header: `Archive ${resourceLabel(r)}?` },
      { buttonLabel: 'Archive', children: <></> }
    )
  const openOnEnter = (e: KeyboardEvent, r: Resource) => {
    if (e.key === 'Enter' && e.target === e.currentTarget) {
      e.preventDefault()
      setOpenedId(r.id)
    }
  }

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-card">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2">
        {switcher}
        <ListToolbarSearch
          value={search}
          onChange={setSearch}
          placeholder={`Search ${lower(type.pluralName)}`}
          aria-label={`Search ${lower(type.pluralName)}`}
          className="w-56"
        />
        <div className="ml-auto flex items-center gap-2">
          {limits.length > MAX_USAGE_COLUMNS && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="icon" variant="ghost" className="h-8 w-8" aria-label="Usage columns">
                  <Columns3Icon size={16} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {productGroups(limits, f => f.product).map((group, _, groups) => (
                  <Fragment key={group.key}>
                    {groups.length > 1 && (
                      <DropdownMenuLabel className="text-xs text-muted-foreground">
                        {group.label}
                      </DropdownMenuLabel>
                    )}
                    {group.items.map(f => {
                      const checked = usage.some(u => u.id === f.id)
                      return (
                        <DropdownMenuCheckboxItem
                          key={f.id}
                          checked={checked}
                          disabled={!checked && usage.length >= MAX_USAGE_COLUMNS}
                          onCheckedChange={() => toggleColumn(f.id)}
                        >
                          {f.name}
                        </DropdownMenuCheckboxItem>
                      )
                    })}
                  </Fragment>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setExplain({ kind: 'adhoc', type, keyValue: '', attributes: {} })}
          >
            Evaluate…
          </Button>
          {canManage && (
            <Button size="sm" variant="outline" hasIcon onClick={() => setAdding(true)}>
              <PlusIcon size={14} /> Add {lower(type.name)}
            </Button>
          )}
        </div>
      </div>

      {isLoading ? (
        <div className="p-4">
          <Skeleton className="h-24 w-full" />
        </div>
      ) : shown.length === 0 ? (
        <PanelEmptyState icon={Boxes} title={needle ? `No matching ${lower(type.pluralName)}` : `No ${lower(type.pluralName)}`} />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{type.name}</TableHead>
              <TableHead>Subscription</TableHead>
              {usage.map(f => (
                <TableHead key={f.id}>{f.name}</TableHead>
              ))}
              <TableHead className="w-px" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {shown.map(r => {
              const cover = coverage(r)
              const count = overrideCount(r)
              return (
                <TableRow
                  key={r.id}
                  tabIndex={0}
                  className="cursor-pointer"
                  onClick={() => setOpenedId(r.id)}
                  onKeyDown={e => openOnEnter(e, r)}
                >
                  <TableCell>
                    <div className="text-sm font-medium">{r.name || r.keyValue}</div>
                    <div className="mt-0.5 flex flex-wrap items-center gap-1">
                      {r.name && <span className="mr-1 font-mono text-xs text-muted-foreground">{r.keyValue}</span>}
                      {Object.keys(r.attributes).length > 0 && <AttributeBadges attributes={r.attributes} />}
                    </div>
                  </TableCell>
                  <TableCell className="text-[13px]" onClick={e => e.stopPropagation()}>
                    <CoverageLinks coverage={cover} />
                  </TableCell>
                  {usage.length > 0 && (
                    <UsageCells type={type} features={usage} snapshot={usageByResource.get(r.id)} />
                  )}
                  <TableCell className="w-px" onClick={e => e.stopPropagation()}>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={`${resourceLabel(r)} actions`}>
                          <MoreVerticalIcon size={16} />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onClick={() => setOpenedId(r.id)}>
                          Overrides…
                          {count > 0 && <span className="ml-auto pl-4 text-xs text-muted-foreground">{count}</span>}
                        </DropdownMenuItem>
                        <DropdownMenuItem onClick={() => setExplain({ kind: 'stored', type, resource: r })}>
                          Why this value…
                        </DropdownMenuItem>
                        {canSubscribe && cover.own.length === 0 && (
                          <DropdownMenuItem
                            onClick={() =>
                              navigate(`${basePath}/subscriptions/create?customerId=${customerId}&resourceId=${r.id}`)
                            }
                          >
                            New subscription
                          </DropdownMenuItem>
                        )}
                        {canManage && (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem onClick={() => setEditing(r)}>Edit</DropdownMenuItem>
                            <DropdownMenuItem disabled={archive.isPending} onClick={() => confirmArchive(r)}>
                              Archive
                            </DropdownMenuItem>
                          </>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}

      {adding && <ResourceDialog customerId={customerId} type={type} onClose={() => setAdding(false)} />}
      {editing && (
        <ResourceDialog customerId={customerId} type={type} resource={editing} onClose={() => setEditing(undefined)} />
      )}
      {opened && (
        <ResourceSheet
          customerId={customerId}
          customerName={customerName}
          type={type}
          resource={opened}
          coverage={coverage(opened)}
          onEdit={() => setEditing(opened)}
          onArchive={() => confirmArchive(opened, () => setOpenedId(undefined))}
          onClose={() => setOpenedId(undefined)}
        />
      )}
      {explain && (
        <ExplainDialog
          customer={{ id: customerId, name: customerName }}
          target={explain}
          onClose={() => setExplain(undefined)}
        />
      )}
    </div>
  )
}

const CoverageLinks = ({ coverage }: { coverage: Coverage }) => {
  const basePath = useBasePath()
  const subs = coverage.own.length ? coverage.own : coverage.customerWide
  if (!subs.length) return <span className="text-muted-foreground">—</span>
  return (
    <div className="flex flex-col gap-0.5">
      {subs.map(s => (
        <span key={s.id} className="flex items-center gap-1.5">
          <Link
            className={cn('font-medium hover:underline', coverage.own.length ? 'text-brand' : 'text-muted-foreground')}
            to={`${basePath}/subscriptions/${s.id}`}
          >
            {s.planName}
          </Link>
          {!coverage.own.length && (
            <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
              Customer
            </Badge>
          )}
        </span>
      ))}
    </div>
  )
}

/** "6 / 8" over a thin bar coloured by how close it is to the limit. */
const UsageBar = ({ level, feature }: { level?: SnapshotLevel; feature?: Feature }) => {
  if (level?.usageUnavailable)
    return <span className="text-muted-foreground">{reasonLabel(CheckReason.USAGE_UNAVAILABLE, undefined)}</span>
  if (level?.used === undefined) return <span className="text-muted-foreground">—</span>
  const v = level.value?.value
  const metered = v?.case === 'meteredValue' ? v.value : undefined
  const limit = metered?.enabled && metered.limit ? Number(metered.limit) : undefined
  const used = Number(level.used)
  const pct = limit === undefined ? undefined : limit > 0 ? (used / limit) * 100 : 100
  const limitText = metered
    ? !metered.enabled
      ? 'Off'
      : metered.limit
        ? formatDecimalGrouped(metered.limit)
        : '∞'
    : level.value
      ? valueLabel(level.value, feature)
      : '∞'
  return (
    <div className="flex min-w-24 flex-col gap-1">
      <span className="text-[13px] tabular-nums">
        {formatDecimalGrouped(level.used)} / {limitText}
      </span>
      {pct !== undefined && (
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
          <div
            className={cn(
              'h-full rounded-full',
              pct >= 100 ? 'bg-destructive' : pct >= 80 ? 'bg-warning' : 'bg-success'
            )}
            style={{ width: `${Math.min(100, pct)}%` }}
          />
        </div>
      )}
    </div>
  )
}

const useResourceSnapshot = (customerId: string, type: ResourceType, resource: Resource) =>
  useQuery(getEntitlementSnapshot, { customerId, resources: resourceSelector(type, resource.keyValue) })

/** The usage columns' features on each resource, by resource id: one batched snapshot per 100 resources. */
const useResourceUsage = (customerId: string, resources: Resource[], features: Feature[]) => {
  const transport = useTransport()
  const featureIds = features.map(f => f.id)
  const ids = featureIds.length ? resources.map(r => r.id) : []
  const batches = Array.from({ length: Math.ceil(ids.length / SNAPSHOT_BATCH) }, (_, i) =>
    ids.slice(i * SNAPSHOT_BATCH, (i + 1) * SNAPSHOT_BATCH)
  )
  const results = useQueries({
    queries: batches.map(resourceIds =>
      createQueryOptions(getEntitlementSnapshots, { customerId, resourceIds, featureIds }, { transport })
    ),
  })
  return new Map(results.flatMap(r => r.data?.snapshots ?? []).map(s => [s.resourceId, s.features]))
}

const UsageCells = ({
  type,
  features,
  snapshot,
}: {
  type: ResourceType
  features: Feature[]
  snapshot?: SnapshotFeature[]
}) => (
  <>
    {features.map(f => (
      <TableCell key={f.id}>
        <UsageBar
          feature={f}
          level={snapshot?.find(sf => sf.feature?.id === f.id)?.levels.find(l => l.resourceTypeId === type.id)}
        />
      </TableCell>
    ))}
  </>
)

/** A resource's subscription, effective values with usage, and its overrides. */
const ResourceSheet = ({
  customerId,
  customerName,
  type,
  resource,
  coverage,
  onEdit,
  onArchive,
  onClose,
}: Scope & {
  resource: Resource
  coverage: Coverage
  onEdit: () => void
  onArchive: () => void
  onClose: () => void
}) => {
  const basePath = useBasePath()
  const { has } = usePermissions()
  const canManage = has(MANAGE_RESOURCES)
  const [explain, setExplain] = useState<Feature>()
  const selector = resourceSelector(type, resource.keyValue)
  const snapshot = useResourceSnapshot(customerId, type, resource)
  const entitled = productRows(snapshot.data?.features ?? [], sf => sf.feature?.product)
  const features = new Map(
    (
      useQuery(listFeatures, {
        pagination: { page: 0, perPage: 500 },
        statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
      }).data?.features ?? []
    ).map(f => [f.id, f])
  )
  const target: Target = { kind: 'stored', type, resource }

  return (
    <Sheet open onOpenChange={open => !open && onClose()}>
      <SheetContent size="medium" className="overflow-y-auto">
        <SheetHeader className="pb-2">
          <div className="flex items-center justify-between gap-2 pr-8">
            <SheetTitle>{resource.name || resource.keyValue}</SheetTitle>
            {canManage && (
              <div className="flex items-center gap-1">
                <Button variant="ghost" size="icon" aria-label="Edit" onClick={onEdit}>
                  <PencilIcon size={16} />
                </Button>
                {!resource.archived && (
                  <Button variant="ghost" size="icon" aria-label="Archive" onClick={onArchive}>
                    <ArchiveIcon size={16} />
                  </Button>
                )}
              </div>
            )}
          </div>
        </SheetHeader>
        <Separator />
        <div className="flex flex-col gap-6 py-4">
          <section className="flex flex-col gap-3">
            <SheetRow label="Type">{type.name}</SheetRow>
            <SheetRow label={keyLabel(type)}>
              <span className="font-mono text-xs">{resource.keyValue}</span>
            </SheetRow>
            {Object.entries(resource.attributes)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([k, v]) => (
                <SheetRow key={k} label={humanize(k)}>
                  {v}
                </SheetRow>
              ))}
            <SheetRow label="Subscription">
              {coverage.own.length || coverage.customerWide.length ? (
                <CoverageLinks coverage={coverage} />
              ) : !has(MANAGE_SUBSCRIPTIONS) ? (
                <span className="text-muted-foreground">—</span>
              ) : (
                <Link
                  className="font-medium text-brand hover:underline"
                  to={`${basePath}/subscriptions/create?customerId=${customerId}&resourceId=${resource.id}`}
                >
                  New subscription
                </Link>
              )}
            </SheetRow>
          </section>

          <Separator />

          <section className="flex flex-col gap-3">
            <h3 className="text-sm font-medium text-muted-foreground">Entitlements</h3>
            {snapshot.isLoading ? (
              <Skeleton className="h-16 w-full" />
            ) : !snapshot.data?.features.length ? (
              <span className="text-sm text-muted-foreground">—</span>
            ) : (
              <div className="overflow-hidden rounded-lg border border-border">
                <Table>
                  <TableBody>
                    {entitled.rows.map((sf, i) => {
                      const header = entitled.headerAt(i)
                      const feature = features.get(sf.feature?.id ?? '')
                      const own = sf.levels.find(l => l.resourceTypeId === type.id)
                      const reason =
                        own?.reason !== undefined && own.reason !== CheckReason.UNSPECIFIED
                          ? reasonLabel(own.reason, type)
                          : undefined
                      return (
                        <Fragment key={sf.feature?.id}>
                          {header && <GroupHeaderRow colSpan={3}>{header}</GroupHeaderRow>}
                          <TableRow>
                            <TableCell className="text-sm font-medium">{sf.feature?.name}</TableCell>
                            <TableCell className="text-[13px]">
                              {reason ? (
                                <span className="text-muted-foreground">{reason}</span>
                              ) : own?.used !== undefined || own?.usageUnavailable ? (
                                <UsageBar level={own} feature={feature} />
                              ) : sf.value ? (
                                valueLabel(sf.value, feature)
                              ) : (
                                <span className="text-muted-foreground">Not entitled</span>
                              )}
                            </TableCell>
                            <TableCell className="w-px">
                              {feature && (
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  className="h-8 w-8"
                                  aria-label={`Why this value for ${feature.name}`}
                                  onClick={() => setExplain(feature)}
                                >
                                  <HelpCircleIcon size={16} />
                                </Button>
                              )}
                            </TableCell>
                          </TableRow>
                        </Fragment>
                      )
                    })}
                  </TableBody>
                </Table>
              </div>
            )}
          </section>

          <Separator />

          <EntitlementRowsEditor
            canEdit={has(MANAGE_ENTITLEMENTS)}
            owner={{ case: 'customerId', value: customerId }}
            resource={{ id: resource.id, type, selector }}
            title="Overrides"
            emptyText="No overrides"
            onEvaluate={setExplain}
          />
        </div>
        {explain && (
          <ExplainDialog
            customer={{ id: customerId, name: customerName }}
            feature={explain}
            target={target}
            onClose={() => setExplain(undefined)}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

const SheetRow = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex items-start gap-2">
    <span className="w-36 shrink-0 text-sm text-muted-foreground">{label}</span>
    <div className="min-w-0 text-sm">{children}</div>
  </div>
)
