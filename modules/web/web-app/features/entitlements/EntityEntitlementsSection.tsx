import { PartialMessage } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Info, Pencil, Plus, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'

import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { entitlementValueLabel } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteEntitlement,
  listEntitlementsByEntity,
  listFeatures,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { Entitlement, EntitlementEntity } from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  entity: PartialMessage<EntitlementEntity>
  hint?: string
  hideHeader?: boolean
  canEdit?: boolean
  addOpen?: boolean
  onAddOpenChange?: (open: boolean) => void
}

export const EntityEntitlementsSection = ({ entity, hint, hideHeader, canEdit = true, addOpen, onAddOpenChange }: Props) => {
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

  const entitlementsQuery = useQuery(listEntitlementsByEntity, { entity: entity as EntitlementEntity })
  const entitlements = entitlementsQuery.data?.entitlements ?? []

  const featuresQuery = useQuery(listFeatures, { pagination: { page: 0, perPage: 100 }, statuses: [] })
  const featureMap = Object.fromEntries(
    (featuresQuery.data?.features ?? []).map(f => [f.id, f])
  )

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [listEntitlementsByEntity.service.typeName] })

  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      invalidate()
      setPendingDelete(null)
    },
  })

  const isFeatureEntity = entity.EntityId?.case === 'featureId'
  const featureEntityId = isFeatureEntity ? (entity.EntityId?.value as string) : undefined
  const featureForEntity = featureEntityId ? featureMap[featureEntityId] : undefined
  const featureEntityIsMetered = featureForEntity?.featureType?.Inner?.case === 'metered'
  const canAdd = !isFeatureEntity || entitlements.length === 0

  const existingFeatureIds = new Set(entitlements.map(e => e.featureId))

  return (
    <div>
      {!hideHeader && (
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-1.5">
            <h3 className="text-sm font-medium">{isFeatureEntity ? 'Entitlement' : 'Entitlements'}</h3>
            {hint && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Info size={14} className="text-muted-foreground cursor-help" />
                  </TooltipTrigger>
                  <TooltipContent side="right" className="max-w-xs">
                    {hint}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
          {canAdd && (
            <Button
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
        <p className="text-sm text-muted-foreground">No entitlements configured.</p>
      ) : (
        <div className="border rounded-md divide-y">
          {entitlements.map(e => (
            <div key={e.id} className="flex items-center justify-between px-3 py-2 text-sm">
              <div>
                <span className="font-medium">
                  {featureMap[e.featureId]?.name ?? e.featureId}
                </span>
              </div>
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <span>{entitlementValueLabel(e.value?.value)}</span>
                {canEdit && (
                  <>
                    <button
                      className="p-1 hover:bg-muted rounded"
                      onClick={() => setSheet({ open: true, existing: e })}
                      title="Edit"
                    >
                      <Pencil size={12} />
                    </button>
                    <button
                      className="p-1 hover:bg-muted rounded text-destructive"
                      onClick={() => setPendingDelete(e)}
                      title="Delete"
                    >
                      <Trash2 size={12} />
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
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
                  Remove <strong>{featureMap[pendingDelete.featureId]?.name ?? pendingDelete.featureId}</strong> entitlement.
                  This cannot be undone.
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
