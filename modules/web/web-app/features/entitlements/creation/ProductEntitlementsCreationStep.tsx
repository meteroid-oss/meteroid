import { Button, Skeleton, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@md/ui'
import { Pencil, Pin, PinOff } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { InheritedIcon } from '@/features/entitlements/InheritedIcon'
import { entitlementTooltip } from '@/features/entitlements/entitlementTooltips'
import { useResolvedEntitlementsForEntity } from '@/features/entitlements/resolved/useResolvedEntitlements'
import { formatResolvedValue } from '@/features/entitlements/utils'
import { env } from '@/lib/env'
import { ResolvedEntitlement } from '@/rpc/api/entitlements/v1/models_pb'

import { EntitlementSpecDialog } from './EntitlementSpecDialog'
import { PendingEntitlementSpec, resolvedToPendingSpec } from './types'

interface Props {
  productId: string
  productName: string
  initialPending?: PendingEntitlementSpec[]
  submitLabel: string
  onBack: () => void
  onConfirm: (specs: PendingEntitlementSpec[]) => void
  isSubmitting?: boolean
}

function specValueLabel(spec: PendingEntitlementSpec): string {
  if (spec.featureType === 'boolean') {
    return spec.boolEnabled !== false ? 'Enabled' : 'Disabled'
  }
  return spec.limit ? `${spec.limit}` : 'Unlimited'
}

export function ProductEntitlementsCreationStep({
  productId,
  productName,
  initialPending = [],
  submitLabel,
  onBack,
  onConfirm,
  isSubmitting,
}: Props) {
  const { entitlements: productEntitlements, isLoading } = useResolvedEntitlementsForEntity({
    type: 'product',
    id: productId,
  })

  const [pending, setPending] = useState<PendingEntitlementSpec[]>(initialPending)
  const [dialog, setDialog] = useState<
    { open: false } | { open: true; spec: PendingEntitlementSpec; index?: number }
  >({ open: false })

  const pendingById = Object.fromEntries(
    pending.filter(s => s.featureId).map(s => [s.featureId!, s])
  )

  const handlePin = (r: ResolvedEntitlement) => {
    const id = r.feature?.id
    if (!id || pendingById[id]) return
    setPending(prev => [...prev, resolvedToPendingSpec(r)])
  }

  const handleUnpin = (featureId: string) => {
    setPending(prev => prev.filter(s => s.featureId !== featureId))
  }

  const handlePinAll = () => {
    const toPin = productEntitlements.filter(r => r.feature?.id && !pendingById[r.feature.id])
    if (toPin.length === 0) {
      toast.info('All entitlements are already pinned.')
      return
    }
    setPending(prev => [...prev, ...toPin.map(r => resolvedToPendingSpec(r))])
  }

  const openOverride = (r: ResolvedEntitlement) => {
    const id = r.feature?.id
    if (!id) return
    const spec = pendingById[id] ?? resolvedToPendingSpec(r)
    const idx = pending.findIndex(s => s.featureId === id)
    setDialog({ open: true, spec, index: idx !== -1 ? idx : undefined })
  }

  const handleDialogSave = (spec: PendingEntitlementSpec) => {
    if (dialog.open && dialog.index !== undefined) {
      setPending(prev => prev.map((s, i) => (i === dialog.index ? spec : s)))
    } else {
      setPending(prev => [...prev, spec])
    }
    setDialog({ open: false })
  }

  const excludeFromDialog =
    dialog.open && dialog.index !== undefined
      ? pending.filter((_, i) => i !== dialog.index)
      : pending

  if (!env.entitlementsEnabled) {
    return (
      <div className="flex gap-2 justify-end pt-4">
        <Button variant="secondary" type="button" onClick={onBack}>← Back</Button>
        <Button variant="primary" type="button" onClick={() => onConfirm([])} disabled={isSubmitting}>
          {submitLabel}
        </Button>
      </div>
    )
  }

  return (
    <TooltipProvider>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium">Entitlements</p>
            <p className="text-xs text-muted-foreground mt-0.5">
              Override or pin entitlements from <strong>{productName}</strong> for this add-on.
            </p>
          </div>
          {productEntitlements.length > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="sm" variant="outline" type="button" onClick={handlePinAll}>
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
            <Skeleton className="h-8 w-full" />
            <Skeleton className="h-8 w-full" />
          </div>
        ) : productEntitlements.length === 0 ? (
          <p className="text-sm text-muted-foreground">No entitlements on this product.</p>
        ) : (
          <div className="border rounded-md divide-y">
            {[...productEntitlements].sort((a, b) => (a.feature?.name ?? '').localeCompare(b.feature?.name ?? '')).map(r => {
              const featureId = r.feature?.id ?? ''
              const pinnedSpec = pendingById[featureId]
              const isPinned = !!pinnedSpec

              return (
                <div key={featureId} className="flex items-center justify-between px-3 py-2 text-sm">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="font-medium truncate">{r.feature?.name ?? featureId}</span>
                    {!isPinned && <InheritedIcon tooltip="Inherited from product" />}
                  </div>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground shrink-0 ml-3">
                    <span>{isPinned ? specValueLabel(pinnedSpec) : formatResolvedValue(r.value)}</span>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          className="p-1 hover:bg-muted rounded"
                          onClick={() => openOverride(r)}
                        >
                          <Pencil size={12} />
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>{entitlementTooltip('add-on', 'override')}</TooltipContent>
                    </Tooltip>
                    {isPinned ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            className="p-1 hover:bg-muted rounded text-destructive"
                            onClick={() => handleUnpin(featureId)}
                          >
                            <PinOff size={12} />
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
                          >
                            <Pin size={12} />
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>{entitlementTooltip('add-on', 'pin')}</TooltipContent>
                      </Tooltip>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        )}

        {dialog.open && (
          <EntitlementSpecDialog
            open={dialog.open}
            onOpenChange={open => !open && setDialog({ open: false })}
            initialSpec={dialog.spec}
            onAdd={handleDialogSave}
            existingEntitlements={excludeFromDialog}
          />
        )}

        <div className="flex gap-2 justify-end pt-2">
          <Button variant="secondary" type="button" onClick={onBack}>← Back</Button>
          <Button
            variant="primary"
            type="button"
            onClick={() => onConfirm(pending)}
            disabled={isSubmitting}
          >
            {submitLabel}
          </Button>
        </div>
      </div>
    </TooltipProvider>
  )
}
