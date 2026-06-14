import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CirclePower, Pencil, Pin, PinOff } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { InheritedIcon } from '@/features/entitlements/InheritedIcon'
import { entitlementTooltip } from '@/features/entitlements/entitlementTooltips'
import {
  toEntitlementEntity,
  useBatchCreateEntitlements,
  useResolvedEntitlementsForEntity,
} from '@/features/entitlements/resolved/useResolvedEntitlements'
import { entitlementValueToSpec, formatResolvedValue, isEntitlementDisabled } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  createEntitlement,
  deleteEntitlement,
  getResolvedEntitlementsForAddOn,
  listEntitlementsByEntity,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { Entitlement, EntitlementValueSchema, ResolvedEntitlement } from '@/rpc/api/entitlements/v1/models_pb';


interface Props {
  addonId: string
}

function isPinnedHere(r: ResolvedEntitlement, addonId: string): boolean {
  const eid = r.origin?.entity?.EntityId
  return !!eid && eid.case === 'addOnId' && eid.value === addonId
}

export const AddonEntitlementsSection = ({ addonId }: Props) => {
  const entity = { type: 'add-on' as const, id: addonId }
  const protoEntity = toEntitlementEntity(entity)

  const { entitlements: resolved, isLoading } = useResolvedEntitlementsForEntity(entity)

  const localQuery = useQuery(listEntitlementsByEntity, { entity: protoEntity })
  const localEntitlements: Entitlement[] = localQuery.data?.entitlements ?? []

  const batchCreate = useBatchCreateEntitlements(entity)

  const qc = useQueryClient()
  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: getResolvedEntitlementsForAddOn.parent,
      cardinality: undefined
    }) })
    void qc.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listEntitlementsByEntity.parent,
      cardinality: undefined
    }) })
  }

  const createMutation = useMutation(createEntitlement, {
    onSuccess: () => {
      invalidate();
      toast.success('Entitlement pinned.')
    },
    onError: err => toast.error(`Failed to pin: ${err.message}`),
  })

  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      invalidate();
      toast.success('Local override removed.')
    },
    onError: err => toast.error(`Failed to remove override: ${err.message}`),
  })

  const updateMutation = useMutation(updateEntitlement, {
    onSuccess: () => {
      invalidate()
    },
    onError: err => toast.error(`Failed to update: ${err.message}`),
  })

  const [dialog, setDialog] = useState<
    { open: false } | { open: true; row: ResolvedEntitlement }
  >({ open: false })

  const handlePin = (r: ResolvedEntitlement) => {
    if (!r.feature?.id) return
    createMutation.mutate({
      featureId: r.feature.id,
      entity: protoEntity,
      value: create(EntitlementValueSchema, entitlementValueToSpec(r.value)),
    })
  }

  const handleUnpin = (r: ResolvedEntitlement) => {
    const local = localEntitlements.find(e => e.featureId === r.feature?.id)
    if (local) deleteMutation.mutate({ id: local.id })
  }

  const handleToggleDisable = (r: ResolvedEntitlement) => {
    const featureId = r.feature?.id
    if (!featureId) return
    const currentlyDisabled = isEntitlementDisabled(r.value)
    let flippedValue: MessageInitShape<typeof EntitlementValueSchema>
    if (r.value.case === 'boolean') {
      flippedValue = { value: { case: 'booleanValue' as const, value: { enabled: currentlyDisabled } } }
    } else if (r.value.case === 'metered') {
      const m = r.value.value
      flippedValue = { value: { case: 'meteredValue' as const,
          value: {
            limit: m.limit,
            resetPeriod: m.resetPeriod,
            enabled: currentlyDisabled
          }
        }
      }
    } else {
      return
    }
    const local = localEntitlements.find(e => e.featureId === featureId)
    if (isPinnedHere(r, addonId) && local) {
      updateMutation.mutate({ id: local.id, value: create(EntitlementValueSchema, flippedValue) })
    } else {
      createMutation.mutate({ featureId, entity: protoEntity, value: create(EntitlementValueSchema, flippedValue) })
    }
  }

  const handlePinAll = () => {
    const unpinned = resolved.filter(r => !isPinnedHere(r, addonId))
    if (unpinned.length === 0) {
      toast.info('All entitlements are already pinned.')
      return
    }
    batchCreate.mutate(
      unpinned.map(r => ({
        featureId: r.feature!.id,
        value: entitlementValueToSpec(r.value),
      }))
    )
  }

  const existingFeatureIds = new Set<string>([
    ...localEntitlements.map(e => e.featureId),
    ...resolved.map(r => r.feature?.id ?? '').filter(Boolean),
  ])

  const dialogRow = dialog.open ? dialog.row : undefined
  const dialogExisting = dialogRow
    ? localEntitlements.find(e => e.featureId === dialogRow.feature?.id)
    : undefined
  const dialogSeed =
    dialogRow && !dialogExisting
      ? create(EntitlementValueSchema, entitlementValueToSpec(dialogRow.value))
      : undefined

  return (
    <TooltipProvider>
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium text-muted-foreground">Entitlements</h3>
          {resolved.length > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={handlePinAll}
                  disabled={batchCreate.isPending}
                >
                  Pin all
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-56">
                {entitlementTooltip('add-on', 'pinAll')}
              </TooltipContent>
            </Tooltip>
          )}
        </div>

        {isLoading ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-8 w-full"/>
            <Skeleton className="h-8 w-full"/>
          </div>
        ) : resolved.length === 0 ? (
          <p className="text-sm text-muted-foreground">No entitlements on this add-on.</p>
        ) : (
          <div className="border rounded-md divide-y">
            {[...resolved].sort((a, b) => (a.feature?.name ?? '').localeCompare(b.feature?.name ?? '')).map(r => {
              const featureId = r.feature?.id ?? ''
              const pinned = isPinnedHere(r, addonId)
              const isBusy = createMutation.isPending || deleteMutation.isPending || updateMutation.isPending
              const isBoolean = r.value.case === 'boolean'
              const isMetered = r.value.case === 'metered'
              const disabled = isEntitlementDisabled(r.value)

              return (
                <div
                  key={featureId}
                  className="flex items-center justify-between px-3 py-2 text-sm"
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="font-medium truncate">{r.feature?.name ?? featureId}</span>
                    {!pinned && <InheritedIcon tooltip="Inherited from product"/>}
                  </div>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground shrink-0 ml-3">
                    <span>{formatResolvedValue(r.value)}</span>
                    {!isBoolean && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="p-1 hover:bg-muted rounded"
                            onClick={() => setDialog({ open: true, row: r })}
                            disabled={isBusy}
                          >
                            <Pencil size={12}/>
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>{entitlementTooltip('add-on', 'override')}</TooltipContent>
                      </Tooltip>
                    )}
                    {pinned ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="p-1 hover:bg-muted rounded text-destructive"
                            onClick={() => handleUnpin(r)}
                            disabled={isBusy}
                          >
                            <PinOff size={12}/>
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>{entitlementTooltip('add-on', 'unpin')}</TooltipContent>
                      </Tooltip>
                    ) : (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="p-1 hover:bg-muted rounded"
                            onClick={() => handlePin(r)}
                            disabled={isBusy}
                          >
                            <Pin size={12}/>
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>{entitlementTooltip('add-on', 'pin')}</TooltipContent>
                      </Tooltip>
                    )}
                    {(isBoolean || isMetered) && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="p-1 hover:bg-muted rounded"
                            onClick={() => handleToggleDisable(r)}
                            disabled={isBusy}
                          >
                            {disabled
                              ? <CirclePower size={12} className="text-destructive"/>
                              : <CirclePower size={12} className="text-primary"/>}
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>{entitlementTooltip('add-on', disabled ? 'enable' : 'disable')}</TooltipContent>
                      </Tooltip>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        )}

        {dialog.open && (
          <EntityEntitlementDialog
            entity={protoEntity}
            existing={dialogExisting}
            seedValue={dialogSeed}
            featureId={dialogRow?.feature?.id}
            featureIsMetered={dialogRow?.value.case === 'metered'}
            existingFeatureIds={existingFeatureIds}
            onClose={() => setDialog({ open: false })}
          />
        )}
      </div>
    </TooltipProvider>
  )
}
