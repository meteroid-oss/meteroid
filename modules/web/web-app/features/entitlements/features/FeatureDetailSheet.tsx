import { skipToken } from '@connectrpc/connect-query';
import {
  Badge,
  Button,
  ScrollArea,
  Separator,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Skeleton,
} from '@md/ui'
import { ArchiveIcon, ArchiveRestoreIcon, ExternalLinkIcon, PencilIcon, PowerIcon } from 'lucide-react'
import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import {
  FeatureActionDialog,
  PendingFeatureAction,
} from '@/features/entitlements/features/statusActions'
import {
  configKindLabel,
  entitlementValueLabel,
  featureKindFromProto,
  featureTypeLabel,
  MeteredFeatureKind,
} from '@/features/entitlements/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { getFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

const MetricLink = ({ kind }: { kind: MeteredFeatureKind }) => {
  const basePath = useBasePath()
  const metricQuery = useQuery(getBillableMetric, { id: kind.metricId })
  const name = metricQuery.data?.billableMetric?.name

  return (
    <Link
      to={`${basePath}/metrics/${kind.metricId}`}
      className="inline-flex items-center gap-1 text-sm font-mono text-primary hover:underline"
    >
      {name ?? kind.metricId}
      <ExternalLinkIcon size={12} />
    </Link>
  )
}

export const FeatureDetailSheet = () => {
  const { featureId } = useParams<{ featureId: string }>()
  const navigate = useNavigate()
  const [pendingAction, setPendingAction] = useState<PendingFeatureAction | null>(null)

  const featureQuery = useQuery(getFeature, featureId ? { id: featureId } : skipToken)
  const feature = featureQuery.data?.feature

  return (
    <>
      <Sheet open onOpenChange={() => navigate('..')}>
        <SheetContent size="medium" onInteractOutside={e => e.preventDefault()}>
          <SheetHeader className="pb-2">
            <SheetTitle>Feature Details</SheetTitle>
            <Separator />
          </SheetHeader>

          {featureQuery.isLoading && (
            <div className="flex flex-col gap-4 py-4">
              <Skeleton className="h-6 w-48" />
              <Skeleton className="h-4 w-64" />
              <Skeleton className="h-4 w-32" />
            </div>
          )}

          {feature && (() => {
            const kind = featureKindFromProto(feature.featureType)
            const isArchived = feature.status === FeatureStatus.ARCHIVED
            return (
              <ScrollArea className="h-[calc(100%-60px)]">
                <div className="flex flex-col gap-6 py-4">
                  <div className="flex items-start justify-between">
                    <div>
                      <h3 className="text-lg font-semibold">{feature.name}</h3>
                      <div className="mt-1 flex items-center gap-2">
                        <Badge variant="secondary">{featureTypeLabel(kind)}</Badge>
                        <FeatureStatusBadge status={feature.status} />
                      </div>
                    </div>
                    <div className="flex items-center gap-1">
                      {isArchived ? (
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setPendingAction({ feature, target: FeatureStatus.ACTIVE })}
                          title="Unarchive feature"
                        >
                          <ArchiveRestoreIcon size={15} />
                        </Button>
                      ) : (
                        <>
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => navigate(`../edit/${feature.id}`)}
                          >
                            <PencilIcon size={15} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() =>
                              setPendingAction({
                                feature,
                                target:
                                  feature.status === FeatureStatus.ACTIVE
                                    ? FeatureStatus.DISABLED
                                    : FeatureStatus.ACTIVE,
                              })
                            }
                            title={feature.status === FeatureStatus.ACTIVE ? 'Disable feature' : 'Enable feature'}
                          >
                            <PowerIcon size={15} />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => setPendingAction({ feature, target: FeatureStatus.ARCHIVED })}
                            title="Archive feature"
                          >
                            <ArchiveIcon size={15} />
                          </Button>
                        </>
                      )}
                    </div>
                  </div>

                  <section>
                    <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                      Feature ID
                    </div>
                    <CopyToClipboardButton text={feature.id} className="max-w-xs" />
                  </section>

                  <section>
                    <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                      Code
                    </div>
                    <CopyToClipboardButton text={feature.code} className="max-w-xs" />
                  </section>

                  {feature.description && (
                    <section>
                      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                        Description
                      </div>
                      <p className="text-sm">{feature.description}</p>
                    </section>
                  )}

                  {kind.type === 'metered' && (
                    <section>
                      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                        Metric
                      </div>
                      <MetricLink kind={kind} />
                    </section>
                  )}

                  {kind.type === 'config' && (
                    <section>
                      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                        Value type
                      </div>
                      <p className="text-sm">{configKindLabel(kind.valueType)}</p>
                    </section>
                  )}

                  {kind.type === 'config' && kind.valueType === 'select' && kind.options && (
                    <section>
                      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                        Options
                      </div>
                      <div className="flex flex-wrap gap-1.5">
                        {kind.options.map(o => (
                          <Badge key={o} variant="secondary">
                            {o}
                          </Badge>
                        ))}
                      </div>
                    </section>
                  )}

                  <section>
                    <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                      Product
                    </div>
                    <p className="text-sm">
                      {feature.product
                        ? feature.product.name
                        : <span className="text-muted-foreground">Global (no product)</span>}
                    </p>
                  </section>

                  <section>
                    <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">
                      Created
                    </div>
                    <p className="text-sm">{parseAndFormatDate(feature.createdAt)}</p>
                  </section>

                  {env.entitlementsEnabled && (
                    <>
                      <Separator />
                      {isArchived ? (
                        // Read-only: the default is hidden from resolution and lists while archived.
                        <div>
                          <h3 className="text-sm font-medium mb-3">Default entitlement</h3>
                          {feature.entitlement ? (
                            <div className="border rounded-md flex items-center justify-between px-3 py-2 text-sm">
                              <span className="font-medium">{feature.name}</span>
                              <span className="text-xs text-muted-foreground">
                                {entitlementValueLabel(feature.entitlement.value?.value)}
                              </span>
                            </div>
                          ) : (
                            <p className="text-sm text-muted-foreground">No entitlements configured.</p>
                          )}
                        </div>
                      ) : (
                        <EntityEntitlementsSection
                          entity={{ EntityId: { case: 'featureId', value: feature.id } }}
                          hint="The default for every customer. Plans, add-ons, and subscriptions can override it. Disabling the feature blocks it everywhere."
                        />
                      )}
                    </>
                  )}
                </div>
              </ScrollArea>
            )
          })()}
        </SheetContent>
      </Sheet>

      <FeatureActionDialog
        pending={pendingAction}
        onClose={() => setPendingAction(null)}
        onDone={() => navigate('..')}
      />
    </>
  )
}
