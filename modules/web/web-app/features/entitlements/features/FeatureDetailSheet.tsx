import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
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
import { useQueryClient } from '@tanstack/react-query'
import { ArchiveRestoreIcon, ExternalLinkIcon, PencilIcon, PowerIcon } from 'lucide-react'
import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'

import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { featureKindFromProto, featureTypeLabel, MeteredFeatureKind } from '@/features/entitlements/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  getFeature,
  listFeatures,
  setFeatureStatus,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
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

type PendingAction = 'toggle' | 'archive' | null

export const FeatureDetailSheet = () => {
  const { featureId } = useParams<{ featureId: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [pendingAction, setPendingAction] = useState<PendingAction>(null)

  const featureQuery = useQuery(getFeature, featureId ? { id: featureId } : disableQuery)
  const feature = featureQuery.data?.feature

  const setStatusMutation = useMutation(setFeatureStatus, {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: [listFeatures.service.typeName] })
      navigate('..')
    },
  })

  const targetStatusForToggle = feature?.status === FeatureStatus.ACTIVE
    ? FeatureStatus.DISABLED
    : FeatureStatus.ACTIVE
  const targetStatus = pendingAction === 'archive'
    ? FeatureStatus.ARCHIVED
    : targetStatusForToggle

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
                        onClick={() => setPendingAction('toggle')}
                        title={feature.status === FeatureStatus.ACTIVE ? 'Disable feature' : 'Re-activate feature'}
                      >
                        <PowerIcon size={15} />
                      </Button>
                      {isArchived && (
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setPendingAction('toggle')}
                          title="Restore (Active)"
                        >
                          <ArchiveRestoreIcon size={15} />
                        </Button>
                      )}
                    </div>
                  </div>

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
                      <EntityEntitlementsSection
                        entity={{ EntityId: { case: 'featureId', value: feature.id } }}
                        hint="The default for every customer. Plans, add-ons, and subscriptions can override it. Disabling the feature blocks it everywhere."
                      />
                    </>
                  )}
                </div>
              </ScrollArea>
            )
          })()}
        </SheetContent>
      </Sheet>

      <AlertDialog open={pendingAction !== null} onOpenChange={open => !open && setPendingAction(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {targetStatus === FeatureStatus.ACTIVE
                ? 'Re-activate feature?'
                : targetStatus === FeatureStatus.DISABLED
                  ? 'Disable feature?'
                  : 'Archive feature?'}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {targetStatus === FeatureStatus.ACTIVE
                ? 'Customers regain access to this feature. All entitlement settings come back as they were.'
                : targetStatus === FeatureStatus.DISABLED
                  ? "Customers will lose access while it's off. Settings stay saved — turning it back on restores everything."
                  : 'Archives the feature and hides it from customers. Settings stay saved for audit and can be brought back later.'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!feature) return
                setStatusMutation.mutate({ id: feature.id, status: targetStatus })
              }}
            >
              Confirm
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
