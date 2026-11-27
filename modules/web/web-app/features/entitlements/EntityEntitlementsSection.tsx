import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Info, Pencil, Plus, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'

import { raisedList, raisedListRow } from '@/components/RaisedList'
import { RowActionButton } from '@/components/RowActionButton'
import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { FeatureStatusBadge } from '@/features/entitlements/features/FeatureStatusBadge'
import { entitlementValueLabel } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteEntitlement,
  getFeature,
  listEntitlementsByEntity,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  Entitlement,
  EntitlementEntity,
  EntitlementEntitySchema,
  FeatureStatus,
} from '@/rpc/api/entitlements/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'

interface Props {
  entity: MessageInitShape<typeof EntitlementEntitySchema>
  hint?: string
  hideHeader?: boolean
  canEdit?: boolean
  addOpen?: boolean
  onAddOpenChange?: (open: boolean) => void
  /** Override the section heading. Defaults to "Default entitlement" for a feature entity, else "Entitlements". */
  title?: string
  /** Shown when the entity has none. */
  emptyText?: string
}

export const EntityEntitlementsSection = ({
  entity,
  hint,
  hideHeader,
  canEdit = true,
  addOpen,
  onAddOpenChange,
  title,
  emptyText = 'No entitlements configured.',
}: Props) => {
  const queryClient = useQueryClient()
  const [sheet, setSheet] = useState<{ open: false } | { open: true; existing?: Entitlement }>({
    open: false,
  })

  useEffect(() => {
    if (addOpen && !sheet.open) setSheet({ open: true })
  }, [addOpen]) // eslint-disable-line react-hooks/exhaustive-deps

  const closeSheet = () => {
    setSheet({ open: false })
    onAddOpenChange?.(false)
  }
  const [pendingDelete, setPendingDelete] = useState<Entitlement | null>(null)

  const entitlementsQuery = useQuery(listEntitlementsByEntity, {
    entity: entity as EntitlementEntity,
  })
  const entitlements = entitlementsQuery.data?.entitlements ?? []

  const isFeatureEntity = entity.EntityId?.case === 'featureId'
  const featureEntityId = isFeatureEntity ? (entity.EntityId?.value as string) : undefined

  const featureMap = Object.fromEntries(
    (entitlementsQuery.data?.features ?? []).map(f => [f.id, f])
  )
  // A feature without a default entitlement is not in that list, but adding one needs its type.
  const featureEntityQuery = useQuery(
    getFeature,
    featureEntityId ? { id: featureEntityId } : skipToken
  )

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listEntitlementsByEntity.parent,
        cardinality: undefined,
      }),
    })

  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      invalidate()
      setPendingDelete(null)
    },
  })

  const featureForEntity = featureEntityQuery.data?.feature
  const featureEntityIsMetered = featureForEntity?.featureType?.Inner?.case === 'metered'
  const canAdd = canEdit && (!isFeatureEntity || entitlements.length === 0)

  const existingFeatureIds = new Set(entitlements.map(e => e.featureId))

  return (
    <div>
      {!hideHeader && (
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-1.5">
            <h3 className="text-sm font-medium">
              {title ?? (isFeatureEntity ? 'Default entitlement' : 'Entitlements')}
            </h3>
            {hint && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Info size={14} className="text-muted-foreground cursor-help" />
                  </TooltipTrigger>
                  <TooltipContent side="right" multiline className="max-w-xs">
                    {hint}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
          {canAdd && (
            <Button
              type="button"
              size="sm"
              variant="secondary"
              hasIcon
              onClick={() => setSheet({ open: true })}
            >
              <Plus size={12} /> Add
            </Button>
          )}
        </div>
      )}

      {entitlementsQuery.isLoading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
        </div>
      ) : entitlements.length === 0 ? (
        <SectionEmptyState className="bg-card dark:bg-foreground/[0.035]">
          {emptyText}
        </SectionEmptyState>
      ) : (
        <div className={raisedList}>
          {entitlements.map(e => {
            const feature = featureMap[e.featureId]
            return (
              <div
                key={e.id}
                className={cn('flex items-center justify-between text-sm', raisedListRow)}
              >
                <div className="flex items-center gap-2">
                  <span className="font-medium">
                    {featureMap[e.featureId]?.name ?? e.featureId}
                  </span>
                  {feature?.status === FeatureStatus.DISABLED && (
                    <FeatureStatusBadge status={feature.status} />
                  )}
                </div>
                <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="mr-1">{entitlementValueLabel(e.value?.value)}</span>
                  {canEdit && (
                    <>
                      <RowActionButton
                        onClick={() => setSheet({ open: true, existing: e })}
                        title="Edit"
                        aria-label="Edit"
                      >
                        <Pencil />
                      </RowActionButton>
                      {!isFeatureEntity && (
                        <RowActionButton
                          destructive
                          onClick={() => setPendingDelete(e)}
                          title="Delete"
                          aria-label="Delete"
                        >
                          <Trash2 />
                        </RowActionButton>
                      )}
                    </>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}

      {sheet.open && (
        <EntityEntitlementDialog
          entity={entity}
          existing={sheet.existing}
          onClose={closeSheet}
          featureId={featureEntityId}
          featureIsMetered={featureEntityIsMetered}
          existingFeatureIds={existingFeatureIds}
        />
      )}

      <AlertDialog open={!!pendingDelete} onOpenChange={open => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove entitlement?</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingDelete && (
                <>
                  Remove{' '}
                  <strong>
                    {featureMap[pendingDelete.featureId]?.name ?? pendingDelete.featureId}
                  </strong>{' '}
                  entitlement. This cannot be undone.
                </>
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => pendingDelete && deleteMutation.mutate({ id: pendingDelete.id })}
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
