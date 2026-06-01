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
import { cn } from '@ui/lib'
import { CirclePower, Pencil, Pin, PinOff } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { InheritedIcon } from '@/features/entitlements/InheritedIcon'
import { entitlementTooltip } from '@/features/entitlements/entitlementTooltips'
import {
  buildInheritanceTooltip,
  entitlementValueToSpec,
  groupByProduct,
  isEntitlementDisabled,
} from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  batchCreateEntitlements,
  createEntitlement,
  deleteEntitlement,
  getEffectiveEntitlements,
  listEntitlementsByEntity,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  EffectiveEntitlement,
  Entitlement,
  EntitlementEntity,
  EntitlementSpec,
  EntitlementValue,
} from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  customerId: string
  /**
   * Subscription id used to (a) classify origin as "Direct" vs "Inherited" and (b) enable the
   * management actions (Override, Pin, Disable, Remove). When omitted, the card is read-only.
   */
  currentSubscriptionId?: string
}

export const EffectiveEntitlementsCard = ({ customerId, currentSubscriptionId }: Props) => {
  const query = useQuery(getEffectiveEntitlements, { customerId })
  const entitlements = query.data?.entitlements ?? []

  // When we have a current subscription, load its direct entitlements so we know which rows are
  // "pinned here" and can look up local row ids for delete/update.
  const subscriptionEntity: EntitlementEntity | undefined = currentSubscriptionId
    ? new EntitlementEntity({
        EntityId: { case: 'subscriptionId', value: currentSubscriptionId },
      })
    : undefined

  const localQuery = useQuery(
    listEntitlementsByEntity,
    subscriptionEntity
      ? { entity: subscriptionEntity }
      : { entity: undefined as unknown as EntitlementEntity },
    { enabled: !!subscriptionEntity }
  )
  const localEntitlements: Entitlement[] = localQuery.data?.entitlements ?? []

  const [dialog, setDialog] = useState<
    | { open: false }
    | { open: true; row?: EffectiveEntitlement }
  >({ open: false })
  const [pinAllOpen, setPinAllOpen] = useState(false)

  const queryClient = useQueryClient()
  const batchCreateMutation = useMutation(batchCreateEntitlements, {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: [listEntitlementsByEntity.service.typeName] })
      queryClient.invalidateQueries({ queryKey: [getEffectiveEntitlements.service.typeName] })
      setPinAllOpen(false)
      toast.success('Entitlements pinned on this subscription.')
    },
    onError: err => toast.error(`Failed to pin entitlements: ${err.message}`),
  })

  if (query.isLoading) {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-12 w-full" />
      </div>
    )
  }

  const showActions = !!currentSubscriptionId

  if (entitlements.length === 0) {
    return (
      <div className="flex flex-col gap-3">
        <p className="text-sm text-muted-foreground">No active entitlements.</p>
        {showActions && (
          <div>
            <Button size="sm" variant="secondary" onClick={() => setDialog({ open: true })}>
              Add entitlement
            </Button>
          </div>
        )}
        {dialog.open && subscriptionEntity && (
          <EntityEntitlementDialog
            entity={subscriptionEntity as PartialMessage<EntitlementEntity>}
            onClose={() => setDialog({ open: false })}
          />
        )}
      </div>
    )
  }

  const groups = groupByProduct(entitlements, e =>
    e.feature?.product ? { id: e.feature.product.id, name: e.feature.product.name } : undefined
  )

  const existingFeatureIds = new Set<string>([
    ...localEntitlements.map(e => e.featureId),
    ...entitlements.map(e => e.feature?.id ?? '').filter(Boolean),
  ])

  // Rows whose winning origin is NOT this subscription — eligible for "Pin all".
  const unpinnedRows = currentSubscriptionId
    ? entitlements.filter(e => {
        const eid = e.origin?.entity?.EntityId
        return !(eid?.case === 'subscriptionId' && eid.value === currentSubscriptionId)
      })
    : []

  const requestPinAll = () => {
    if (!subscriptionEntity) return
    if (unpinnedRows.length === 0) {
      toast.info('All entitlements are already pinned on this subscription.')
      return
    }
    setPinAllOpen(true)
  }

  const confirmPinAll = () => {
    if (!subscriptionEntity) return
    batchCreateMutation.mutate({
      entity: subscriptionEntity,
      specs: unpinnedRows
        .filter(r => !!r.feature?.id)
        .map(
          r =>
            new EntitlementSpec({
              featureId: r.feature!.id,
              value: new EntitlementValue(entitlementValueToSpec(r.value)),
            })
        ),
    })
  }

  return (
    <TooltipProvider>
      <div className="flex flex-col gap-4">
        {showActions && (
          <div className="flex items-center justify-end gap-2">
            <Button size="sm" variant="secondary" onClick={() => setDialog({ open: true })}>
              Add entitlement
            </Button>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={requestPinAll}
                  disabled={batchCreateMutation.isPending}
                >
                  Pin all
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-56">
                Save local copies of every upstream entitlement on this subscription.
                Already-pinned entries are skipped.
              </TooltipContent>
            </Tooltip>
          </div>
        )}
        {[...groups]
          .sort((a, b) => {
            if (!a.id) return 1
            if (!b.id) return -1
            return a.name.localeCompare(b.name)
          })
          .map(group => (
          <div key={group.id ?? '__general__'}>
            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-1 px-1">
              {group.name}
            </p>
            <div className="border border-border rounded-lg divide-y divide-border overflow-hidden">
              {[...group.items]
                .sort((a, b) => (a.feature?.name ?? '').localeCompare(b.feature?.name ?? ''))
                .map((e, i) => (
                <EntitlementRow
                  key={i}
                  entitlement={e}
                  currentSubscriptionId={currentSubscriptionId}
                  subscriptionEntity={subscriptionEntity}
                  localEntitlements={localEntitlements}
                  onOverride={row => setDialog({ open: true, row })}
                />
              ))}
            </div>
          </div>
        ))}

        {dialog.open && subscriptionEntity && (() => {
          const existing = dialog.row
            ? localEntitlements.find(e => e.featureId === dialog.row?.feature?.id)
            : undefined
          // Inherited row → seed the dialog with the resolved value so the user adjusts
          // an "Override" form pre-filled with current values (instead of an empty "Add" form).
          const seedValue =
            dialog.row && !existing
              ? new EntitlementValue(entitlementValueToSpec(dialog.row.value))
              : undefined
          return (
            <EntityEntitlementDialog
              entity={subscriptionEntity as PartialMessage<EntitlementEntity>}
              existing={existing}
              seedValue={seedValue}
              featureId={dialog.row?.feature?.id}
              existingFeatureIds={existingFeatureIds}
              onClose={() => setDialog({ open: false })}
            />
          )
        })()}

        <AlertDialog open={pinAllOpen} onOpenChange={setPinAllOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Pin all entitlements?</AlertDialogTitle>
              <AlertDialogDescription>
                {`This will save local copies of ${unpinnedRows.length} inherited ${
                  unpinnedRows.length === 1 ? 'entitlement' : 'entitlements'
                } on this subscription. Pinned values stay fixed even if upstream changes.`}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction onClick={confirmPinAll}>Pin all</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </TooltipProvider>
  )
}

interface RowProps {
  entitlement: EffectiveEntitlement
  currentSubscriptionId?: string
  subscriptionEntity?: EntitlementEntity
  localEntitlements: Entitlement[]
  onOverride: (row: EffectiveEntitlement) => void
}

/**
 * Inheritance indicator — renders nothing when the row is pinned directly on the current
 * subscription; otherwise shows the shared `<InheritedIcon>` with a tooltip describing the
 * actual origin (plan / plan version / add-on / global feature).
 */
const InheritedIndicator = ({
  entitlement,
  currentSubscriptionId,
}: {
  entitlement: EffectiveEntitlement
  currentSubscriptionId?: string
}) => {
  const entity = entitlement.origin?.entity?.EntityId
  if (!entity) return null

  const isDirect =
    entity.case === 'subscriptionId' &&
    currentSubscriptionId !== undefined &&
    entity.value === currentSubscriptionId
  if (isDirect) return null

  return (
    <InheritedIcon
      tooltip={buildInheritanceTooltip(entitlement.origin, entitlement.feature?.product)}
    />
  )
}

/**
 * Per-row management actions. Mirrors `RowActions` in `ResolvedEntitlementsPanel` but is
 * always scoped to the current subscription (no entity-type branching needed).
 */
const RowActions = ({
  entitlement,
  subscriptionEntity,
  pinnedHere,
  localEntitlements,
  onOverride,
}: {
  entitlement: EffectiveEntitlement
  subscriptionEntity: EntitlementEntity
  pinnedHere: boolean
  localEntitlements: Entitlement[]
  onOverride: (row: EffectiveEntitlement) => void
}) => {
  const featureId = entitlement.feature?.id
  const value = entitlement.value
  const isDisabled = isEntitlementDisabled(value)

  const queryClient = useQueryClient()
  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: [listEntitlementsByEntity.service.typeName] })
    queryClient.invalidateQueries({ queryKey: [getEffectiveEntitlements.service.typeName] })
  }

  const createMutation = useMutation(createEntitlement, {
    onSuccess: () => {
      invalidate()
      toast.success('Entitlement pinned on this subscription.')
    },
    onError: err => toast.error(`Failed to pin entitlement: ${err.message}`),
  })
  const updateMutation = useMutation(updateEntitlement, {
    onSuccess: () => invalidate(),
    onError: err => toast.error(`Failed to update entitlement: ${err.message}`),
  })
  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      invalidate()
      toast.success('Local override removed.')
    },
    onError: err => toast.error(`Failed to remove override: ${err.message}`),
  })

  const isBusy =
    createMutation.isPending || updateMutation.isPending || deleteMutation.isPending

  const handlePin = () => {
    if (!featureId) return
    createMutation.mutate({
      featureId,
      entity: subscriptionEntity,
      value: new EntitlementValue(entitlementValueToSpec(value)),
    })
  }

  const handleToggleDisable = () => {
    if (!featureId) return
    let flipped: ConstructorParameters<typeof EntitlementValue>[0]
    if (value.case === 'boolean') {
      flipped = {
        value: {
          case: 'booleanValue' as const,
          value: { enabled: !value.value.enabled },
        },
      }
    } else if (value.case === 'metered') {
      const m = value.value
      flipped = {
        value: {
          case: 'meteredValue' as const,
          value: {
            limit: m.limit,
            resetPeriod: m.resetPeriod,
            enabled: !m.enabled,
          },
        },
      }
    } else {
      return
    }

    if (pinnedHere) {
      const local = localEntitlements.find(e => e.featureId === featureId)
      if (local) {
        updateMutation.mutate({ id: local.id, value: new EntitlementValue(flipped) })
        return
      }
    }
    createMutation.mutate({
      featureId,
      entity: subscriptionEntity,
      value: new EntitlementValue(flipped),
    })
  }

  const handleRemove = () => {
    const local = localEntitlements.find(e => e.featureId === featureId)
    if (local) deleteMutation.mutate({ id: local.id })
  }

  const isBoolean = value.case === 'boolean'
  const isMetered = value.case === 'metered'

  return (
    <div className="flex items-center gap-1">
      {!isBoolean && (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="p-1 hover:bg-muted rounded"
              onClick={() => onOverride(entitlement)}
              disabled={isBusy}
            >
              <Pencil size={12} />
            </button>
          </TooltipTrigger>
          <TooltipContent>{entitlementTooltip('subscription', 'override')}</TooltipContent>
        </Tooltip>
      )}

      {pinnedHere ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="p-1 hover:bg-muted rounded text-destructive"
              onClick={handleRemove}
              disabled={isBusy}
            >
              <PinOff size={12} />
            </button>
          </TooltipTrigger>
          <TooltipContent>{entitlementTooltip('subscription', 'unpin')}</TooltipContent>
        </Tooltip>
      ) : (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="p-1 hover:bg-muted rounded"
              onClick={handlePin}
              disabled={isBusy}
            >
              <Pin size={12} />
            </button>
          </TooltipTrigger>
          <TooltipContent>{entitlementTooltip('subscription', 'pin')}</TooltipContent>
        </Tooltip>
      )}

      {(isBoolean || isMetered) && (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="p-1 hover:bg-muted rounded"
              onClick={handleToggleDisable}
              disabled={isBusy}
            >
              {isDisabled
                ? <CirclePower size={12} className="text-destructive" />
                : <CirclePower size={12} className="text-primary" />}
            </button>
          </TooltipTrigger>
          <TooltipContent>{entitlementTooltip('subscription', isDisabled ? 'enable' : 'disable')}</TooltipContent>
        </Tooltip>
      )}
    </div>
  )
}

const EntitlementRow = ({
  entitlement,
  currentSubscriptionId,
  subscriptionEntity,
  localEntitlements,
  onOverride,
}: RowProps) => {
  const { value } = entitlement
  const featureName = entitlement.feature?.name ?? entitlement.feature?.id ?? ''
  const isDisabled = isEntitlementDisabled(value)

  const originEntity = entitlement.origin?.entity?.EntityId
  const pinnedHere =
    !!currentSubscriptionId &&
    originEntity?.case === 'subscriptionId' &&
    originEntity.value === currentSubscriptionId

  const actions = subscriptionEntity ? (
    <RowActions
      entitlement={entitlement}
      subscriptionEntity={subscriptionEntity}
      pinnedHere={pinnedHere}
      localEntitlements={localEntitlements}
      onOverride={onOverride}
    />
  ) : null

  let valueLabel: string
  if (value.case === 'boolean') {
    valueLabel = value.value.enabled ? 'Enabled' : 'Disabled'
  } else if (value.case === 'metered') {
    const m = value.value
    const consumed = m.consumed ? Number(m.consumed) : undefined
    const limit = m.limit ? Number(m.limit) : undefined
    valueLabel = consumed !== undefined
      ? `${consumed.toLocaleString()} / ${limit !== undefined ? limit.toLocaleString() : '∞'}`
      : limit !== undefined ? limit.toLocaleString() : 'Unlimited'
  } else {
    valueLabel = '—'
  }

  return (
    <div
      className={cn(
        'group flex items-center justify-between px-4 py-2.5 text-sm',
        isDisabled && 'opacity-60'
      )}
    >
      <div className="flex items-center gap-2 min-w-0">
        <span className="font-medium truncate">{featureName}</span>
        <InheritedIndicator
          entitlement={entitlement}
          currentSubscriptionId={currentSubscriptionId}
        />
      </div>
      <div className="flex items-center gap-2 shrink-0 ml-4">
        <span className="text-muted-foreground text-xs tabular-nums">{valueLabel}</span>
        {actions}
      </div>
    </div>
  )
}
