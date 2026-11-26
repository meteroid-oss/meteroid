import { skipToken } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Skeleton,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@md/ui'
import { ArchiveIcon, ArchiveRestoreIcon, PencilIcon, PowerIcon } from 'lucide-react'
import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'

import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { ExplainDialog } from '@/features/entitlements/explain/ExplainDialog'
import { FeatureLimitQuantity } from '@/features/entitlements/features/FeatureLimitQuantity'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { FeatureUsedBy } from '@/features/entitlements/features/FeatureUsedBy'
import {
  capitalize,
  KIND_ICON,
  KIND_NAME,
  kindKey,
} from '@/features/entitlements/features/featureKind'
import {
  FeatureActionDialog,
  PendingFeatureAction,
} from '@/features/entitlements/features/statusActions'
import { appliesPerLabel, isAppCounted, kindLabel, policyLabel } from '@/features/entitlements/model'
import { EntitlementRowsEditor } from '@/features/entitlements/rows/EntitlementRowsEditor'
import { useResourceTypes } from '@/features/resources/hooks'
import { useBasePath } from '@/hooks/useBasePath'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { getFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  type Feature,
  FeatureKind,
  FeatureStatus,
  FeatureValueType,
} from '@/rpc/api/entitlements/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

const DetailRow = ({ label, children }: { label: React.ReactNode; children: React.ReactNode }) => (
  <div className="flex items-start gap-2">
    <span className="w-36 shrink-0 text-sm text-muted-foreground">{label}</span>
    <div className="min-w-0 text-sm">{children}</div>
  </div>
)

const MetricLink = ({ metricId }: { metricId: string }) => {
  const basePath = useBasePath()
  const name = useQuery(getBillableMetric, { id: metricId }).data?.billableMetric?.name
  return (
    <Link to={`${basePath}/metrics/${metricId}`} className="font-medium text-brand hover:underline">
      {name ?? metricId}
    </Link>
  )
}

const FeatureDetails = ({ feature, readOnly }: { feature: Feature; readOnly: boolean }) => {
  const { byId: typesById, hasTypes } = useResourceTypes()
  const kind = kindKey(feature.kind)
  const KindIcon = KIND_ICON[kind]
  const isLimit = feature.kind === FeatureKind.LIMIT
  const isValue = feature.kind === FeatureKind.VALUE
  const options = isValue && feature.value?.type === FeatureValueType.SELECT ? feature.value.options : []

  return (
    <div className="flex flex-col gap-3">
      <DetailRow label="ID">
        <LocalId localId={feature.id} className="max-w-48" />
      </DetailRow>
      <DetailRow label="Kind">
        <span className="inline-flex items-center gap-1.5">
          <KindIcon size={14} className="text-muted-foreground" />
          {KIND_NAME[kind]}
        </span>
      </DetailRow>
      {isValue && <DetailRow label="Value type">{kindLabel(feature)}</DetailRow>}
      {options.length > 0 && (
        <DetailRow label={feature.value?.ordered ? 'Options (lowest first)' : 'Options'}>
          <div className="flex flex-wrap gap-1.5">
            {options.map(o => (
              <Badge key={o} variant="secondary" size="sm">
                {o}
              </Badge>
            ))}
          </div>
        </DetailRow>
      )}
      {(isLimit || isValue) && <DetailRow label="Unit">{feature.unit || '—'}</DetailRow>}
      {isLimit && (
        <DetailRow label="Counted by">
          {feature.limit?.metricId ? (
            <MetricLink metricId={feature.limit.metricId} />
          ) : isAppCounted(feature) ? (
            'Your app'
          ) : (
            '—'
          )}
        </DetailRow>
      )}
      {hasTypes && <DetailRow label="Applies per">{appliesPerLabel(feature, typesById)}</DetailRow>}
      {isLimit && (
        <DetailRow label="Policy">{capitalize(policyLabel(feature.policy, isAppCounted(feature)))}</DetailRow>
      )}
      {isLimit && env.entitlementsEnabled && (
        <FeatureLimitQuantity featureId={feature.id} readOnly={readOnly} Row={DetailRow} />
      )}
      <DetailRow label="Product">{feature.product?.name ?? '—'}</DetailRow>
      <DetailRow label="Created">{parseAndFormatDate(feature.createdAt)}</DetailRow>
      <DetailRow label="Description">
        <span className="whitespace-pre-wrap">{feature.description || '—'}</span>
      </DetailRow>
    </div>
  )
}

export const FeatureDetailSheet = () => {
  const { featureId } = useParams<{ featureId: string }>()
  const navigate = useNavigate()
  const { has } = usePermissions()
  const [pendingAction, setPendingAction] = useState<PendingFeatureAction | null>(null)
  const [tab, setTab] = useState('details')
  const [evaluating, setEvaluating] = useState(false)

  const featureQuery = useQuery(getFeature, featureId ? { id: featureId } : skipToken)
  const feature = featureQuery.data?.feature
  const isArchived = feature?.status === FeatureStatus.ARCHIVED

  return (
    <>
      <Sheet open onOpenChange={() => navigate('..')}>
        <SheetContent size="medium">
          <SheetHeader className="pb-2">
            {feature ? (
              <>
                <div className="flex items-center justify-between gap-2 pr-8">
                  <div className="flex min-w-0 items-center gap-2">
                    <SheetTitle className="truncate">{feature.name}</SheetTitle>
                    <FeatureStatusBadge status={feature.status} />
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {env.entitlementsEnabled && !isArchived && (
                      <Button size="sm" variant="ghost" onClick={() => setEvaluating(true)}>
                        Evaluate…
                      </Button>
                    )}
                    <Gate perm="catalog:manage">
                      {isArchived ? (
                        <Button
                          variant="ghost"
                          size="icon"
                          title="Restore"
                          aria-label="Restore"
                          onClick={() => setPendingAction({ feature, target: FeatureStatus.ACTIVE })}
                        >
                          <ArchiveRestoreIcon size={16} />
                        </Button>
                      ) : (
                        <>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Edit"
                            aria-label="Edit"
                            onClick={() => navigate(`../edit/${feature.id}`)}
                          >
                            <PencilIcon size={16} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title={feature.status === FeatureStatus.ACTIVE ? 'Disable' : 'Enable'}
                            aria-label={feature.status === FeatureStatus.ACTIVE ? 'Disable' : 'Enable'}
                            onClick={() =>
                              setPendingAction({
                                feature,
                                target:
                                  feature.status === FeatureStatus.ACTIVE
                                    ? FeatureStatus.DISABLED
                                    : FeatureStatus.ACTIVE,
                              })
                            }
                          >
                            <PowerIcon size={16} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Archive"
                            aria-label="Archive"
                            onClick={() => setPendingAction({ feature, target: FeatureStatus.ARCHIVED })}
                          >
                            <ArchiveIcon size={16} />
                          </Button>
                        </>
                      )}
                    </Gate>
                  </div>
                </div>
                <div className="flex">
                  <LocalId localId={feature.code} className="font-mono" />
                </div>
              </>
            ) : (
              <>
                <SheetTitle className="sr-only">Feature</SheetTitle>
                {featureQuery.isLoading && (
                  <div className="flex flex-col gap-2">
                    <Skeleton className="h-7 w-48" />
                    <Skeleton className="h-6 w-32" />
                  </div>
                )}
              </>
            )}
            <Separator />
          </SheetHeader>

          {featureQuery.isLoading && (
            <div className="flex flex-col gap-3 py-4">
              <Skeleton className="h-4 w-64" />
              <Skeleton className="h-4 w-56" />
              <Skeleton className="h-4 w-48" />
              <Skeleton className="h-4 w-60" />
            </div>
          )}

          {!featureQuery.isLoading && !feature && <PanelEmptyState title="Feature not found" />}

          {feature && (
            <Tabs value={tab} onValueChange={setTab} className="flex flex-col gap-4 py-2">
              {env.entitlementsEnabled && (
                <TabsList variant="ghost">
                  <TabsTrigger value="details">Details</TabsTrigger>
                  <TabsTrigger value="usedBy">Used by</TabsTrigger>
                  <TabsTrigger value="defaults">Defaults</TabsTrigger>
                </TabsList>
              )}
              <TabsContent value="details" className="mt-0">
                <FeatureDetails feature={feature} readOnly={isArchived} />
              </TabsContent>
              {env.entitlementsEnabled && (
                <>
                  <TabsContent value="usedBy" className="mt-0">
                    <FeatureUsedBy feature={feature} />
                  </TabsContent>
                  <TabsContent value="defaults" className="mt-0">
                    <EntitlementRowsEditor
                      owner={{ case: 'featureId', value: feature.id }}
                      feature={feature}
                      canEdit={!isArchived && has('catalog:manage')}
                      title={
                        feature.product
                          ? `Default for ${feature.product.name} subscribers`
                          : 'Default for all customers'
                      }
                      emptyText="No default"
                    />
                  </TabsContent>
                </>
              )}
            </Tabs>
          )}
        </SheetContent>
      </Sheet>

      {evaluating && feature && <ExplainDialog feature={feature} onClose={() => setEvaluating(false)} />}

      <FeatureActionDialog
        pending={pendingAction}
        onClose={() => setPendingAction(null)}
        onDone={() => navigate('..')}
      />
    </>
  )
}
