import {
  Badge,
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
  TabsContent,
  TabsList,
  TabsTrigger,
  cn,
} from '@md/ui'
import { Boxes } from 'lucide-react'
import { Link } from 'react-router'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { AttributionPreviewRow } from '@/features/productCatalog/metrics/AttributionPreview'
import { lower } from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listCustomersByIds } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { PlanStatus } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listResources } from '@/rpc/api/resources/v1/resources-ResourcesService_connectquery'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

const useAttributedMetrics = (type: ResourceType) =>
  (useQuery(listBillableMetrics, { pagination: { page: 0, perPage: 500 } }).data?.billableMetrics ?? []).filter(
    m => m.attributedTo?.resourceTypeId === type.id
  )

/** Where each metric attributed to the type would send its last 7 days, read from `canonical`. */
export const TypeCoverage = ({ type, canonical }: { type: ResourceType; canonical: string }) => {
  const metrics = useAttributedMetrics(type)
  if (!metrics.length) return null
  return (
    <div className="space-y-4">
      <h3 className="font-semibold">Attributed metrics, last 7 days</h3>
      <div className="overflow-hidden rounded-lg border border-border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Metric</TableHead>
              <TableHead className="text-right">{type.name}</TableHead>
              <TableHead className="text-right">Customer</TableHead>
              <TableHead className="text-right">Unattributed</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {metrics.map(m => (
              <AttributionPreviewRow
                key={m.id}
                metricId={m.id}
                metricName={m.name}
                value={{
                  typeId: type.id,
                  property: m.attributedTo?.property ?? (canonical === type.keys[0] ? undefined : canonical),
                }}
              />
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  )
}

/** Every stored resource of the type, across customers. */
export const TypeResources = ({ type }: { type: ResourceType }) => {
  const basePath = useBasePath()
  const query = useQuery(listResources, {
    resourceType: type.id,
    includeArchived: true,
    pagination: { page: 0, perPage: 500 },
  })
  const resources = query.data?.resources ?? []
  const customerIds = [...new Set(resources.map(r => r.customerId))]
  const customers = new Map(
    (useQuery(listCustomersByIds, { ids: customerIds }, { enabled: customerIds.length > 0 }).data?.customers ?? []).map(
      c => [c.id, c.name]
    )
  )
  if (query.isLoading) return <Skeleton className="h-24 w-full" />
  if (!resources.length) return <PanelEmptyState icon={Boxes} title={`No ${lower(type.pluralName)}`} />
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{type.name}</TableHead>
            <TableHead>Customer</TableHead>
            <TableHead>Attributes</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {resources.map(r => (
            <TableRow key={r.id} className={cn(r.archived && 'opacity-60')}>
              <TableCell>
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium">{r.name || r.keyValue}</span>
                  {r.archived && (
                    <Badge variant="outline" size="sm">
                      Archived
                    </Badge>
                  )}
                </div>
                <span className="font-mono text-xs text-muted-foreground">{r.keyValue}</span>
              </TableCell>
              <TableCell className="text-[13px]">
                <Link className="font-medium text-brand hover:underline" to={`${basePath}/customers/${r.customerId}`}>
                  {customers.get(r.customerId) ?? r.customerId}
                </Link>
              </TableCell>
              <TableCell>
                <AttributeBadges attributes={r.attributes} />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

export const AttributeBadges = ({ attributes }: { attributes: Record<string, string> }) => {
  const entries = Object.entries(attributes).sort(([a], [b]) => a.localeCompare(b))
  if (!entries.length) return <span className="text-muted-foreground">—</span>
  return (
    <div className="flex flex-wrap gap-1">
      {entries.map(([k, v]) => (
        <Badge key={k} variant="outline" size="sm" className="font-normal" title={k}>
          {v}
        </Badge>
      ))}
    </div>
  )
}

/** The metrics attributed to the type, the features applying per it and the plans sold for it. */
export const TypeUsedBy = ({ type }: { type: ResourceType }) => {
  const basePath = useBasePath()
  const metrics = useAttributedMetrics(type)
  const features =
    useQuery(listFeatures, { levelResourceTypeId: type.id, pagination: { page: 0, perPage: 500 } }).data?.features ?? []
  const plans = (
    useQuery(listPlans, { filters: { statuses: [PlanStatus.ACTIVE, PlanStatus.INACTIVE], types: [] } }).data?.plans ?? []
  ).filter(p => p.soldForResourceTypeId === type.id)
  const groups = [
    { title: 'Attributed metrics', items: metrics.map(m => ({ id: m.id, name: m.name, to: `metrics/${m.id}` })) },
    {
      title: `Features applying per ${lower(type.name)}`,
      items: features.map(f => ({ id: f.id, name: f.name, to: `features/${f.id}` })),
    },
    { title: 'Plans sold for it', items: plans.map(p => ({ id: p.id, name: p.name, to: `plans/${p.id}` })) },
  ]
  return (
    <div className="flex flex-col gap-6">
      {groups.map(g => (
        <section key={g.title} className="flex flex-col gap-3">
          <h3 className="text-sm font-medium text-muted-foreground">{g.title}</h3>
          {g.items.length ? (
            <div className="flex flex-wrap gap-1.5">
              {g.items.map(i => (
                <Link key={i.id} to={`${basePath}/${i.to}`}>
                  <Badge variant="outline" className="font-normal hover:bg-accent">
                    {i.name}
                  </Badge>
                </Link>
              ))}
            </div>
          ) : (
            <span className="text-sm text-muted-foreground">—</span>
          )}
        </section>
      ))}
    </div>
  )
}

export const TypeDetailSheet = ({ type, onClose }: { type: ResourceType; onClose: () => void }) => (
  <Sheet open onOpenChange={open => !open && onClose()}>
    <SheetContent size="medium" className="overflow-y-auto">
      <SheetHeader className="pb-2">
        <SheetTitle>{type.name}</SheetTitle>
        <span className="font-mono text-xs text-muted-foreground">{type.code}</span>
      </SheetHeader>
      <Separator />
      <Tabs defaultValue="resources" className="flex flex-col gap-4 py-4">
        <TabsList variant="ghost" className="self-start">
          <TabsTrigger value="resources">{type.pluralName}</TabsTrigger>
          <TabsTrigger value="used">Used by</TabsTrigger>
        </TabsList>
        <TabsContent value="resources" className="mt-0">
          <TypeResources type={type} />
        </TabsContent>
        <TabsContent value="used" className="mt-0">
          <TypeUsedBy type={type} />
        </TabsContent>
      </Tabs>
    </SheetContent>
  </Sheet>
)
