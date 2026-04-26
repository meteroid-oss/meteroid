/**
 * ResolvedEntitlementsPanel — read-only + action panel for resolved entitlements.
 *
 * Displays the full resolved entitlement list for any entity (product, add-on,
 * plan-version, subscription, quote), grouped by product.  Each row shows the
 * winning value and its origin layer, with a kebab menu for Override / Pin /
 * Disable / Remove-local-override actions.
 */
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
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { cn } from '@ui/lib'
import { MoreVerticalIcon } from 'lucide-react'
import { FC, forwardRef, useImperativeHandle, useState } from 'react'
import { toast } from 'sonner'

import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { InheritedIcon } from '@/features/entitlements/InheritedIcon'
import {
  buildInheritanceTooltip,
  entitlementValueToSpec,
  formatResolvedValue,
  groupByProduct,
  isEntitlementDisabled,
} from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  createEntitlement,
  deleteEntitlement,
  listEntitlementsByEntity,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  Entitlement,
  EntitlementEntity,
  EntitlementValue,
  ResolvedEntitlement,
} from '@/rpc/api/entitlements/v1/models_pb'

import {
  toEntitlementEntity,
  useBatchCreateEntitlements,
  useResolvedEntitlementsForEntity,
  type PersistedEntity,
} from './useResolvedEntitlements'

// ── Helpers ───────────────────────────────────────────────────────────────────

const ENTITY_LABEL: Record<PersistedEntity['type'], string> = {
  product: 'product',
  'add-on': 'add-on',
  'plan-version': 'plan version',
  subscription: 'subscription',
  quote: 'quote',
}

/**
 * Return true when the resolved row is pinned directly on `entity` (the origin
 * matches the entity type + id).
 */
function isPinnedHere(r: ResolvedEntitlement, entity: PersistedEntity): boolean {
  const eid = r.origin?.entity?.EntityId
  if (!eid || eid.case === undefined) return false
  switch (entity.type) {
    case 'add-on':
      return eid.case === 'addOnId' && eid.value === entity.id
    case 'plan-version':
      return eid.case === 'planVersionId' && eid.value === entity.id
    case 'subscription':
      return eid.case === 'subscriptionId' && eid.value === entity.id
    case 'quote':
      return eid.case === 'quoteId' && eid.value === entity.id
    case 'product':
      // Product has no EntitlementEntity variant — treat as never pinned here
      return false
  }
}

// ── Props ─────────────────────────────────────────────────────────────────────

type Props = {
  entity: PersistedEntity
  /**
   * Whether this context allows pinning upstream entitlements down to the
   * current entity.  Should be false for Product and Add-on (they are the top
   * of the hierarchy and there is nothing above them to inherit from).
   */
  canPin: boolean
  /**
   * Suppress the in-panel "Add entitlement" button. Useful when the parent
   * page wants to render its own add trigger in a section header. Pair with
   * the `openAdd()` method on the ref.
   */
  hideAddButton?: boolean
}

export type ResolvedEntitlementsPanelHandle = {
  /** Imperatively open the Add-entitlement dialog. */
  openAdd: () => void
}

// ── RowActions subcomponent ───────────────────────────────────────────────────

type RowActionsProps = {
  row: ResolvedEntitlement
  entity: PersistedEntity
  label: string
  canPin: boolean
  pinnedHere: boolean
  /** Full list of direct entity entitlements — used to look up the local row id for deletion */
  localEntitlements: Entitlement[]
  onOverride: (row: ResolvedEntitlement) => void
  onInvalidate: () => void
}

const RowActions: FC<RowActionsProps> = ({
  row,
  entity,
  label,
  canPin,
  pinnedHere,
  localEntitlements,
  onOverride,
  onInvalidate,
}) => {
  const featureName = row.feature?.name ?? row.feature?.id ?? ''
  const disabled = isEntitlementDisabled(row.value)

  // ── Mutations ──────────────────────────────────────────────────────────────

  const createMutation = useMutation(createEntitlement, {
    onSuccess: () => {
      onInvalidate()
      toast.success(`Entitlement pinned on this ${label}.`)
    },
    onError: err => toast.error(`Failed to pin entitlement: ${err.message}`),
  })

  const updateMutation = useMutation(updateEntitlement, {
    onSuccess: () => {
      onInvalidate()
    },
    onError: err => toast.error(`Failed to update entitlement: ${err.message}`),
  })

  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      onInvalidate()
      toast.success('Local override removed.')
    },
    onError: err => toast.error(`Failed to remove override: ${err.message}`),
  })

  // ── Action handlers ────────────────────────────────────────────────────────

  const handlePin = () => {
    if (entity.type === 'product') return
    const protoEntity = toEntitlementEntity(entity)
    createMutation.mutate({
      featureId: row.feature!.id,
      entity: protoEntity,
      value: new EntitlementValue(entitlementValueToSpec(row.value)),
    })
  }

  const handleToggleDisable = () => {
    const featureId = row.feature?.id
    if (!featureId || entity.type === 'product') return

    const protoEntity = toEntitlementEntity(entity)
    // Build the flipped value directly from the resolved row's typed variants
    let flippedValue: ConstructorParameters<typeof EntitlementValue>[0]
    if (row.value.case === 'boolean') {
      flippedValue = {
        value: {
          case: 'booleanValue' as const,
          value: { enabled: !row.value.value.enabled },
        },
      }
    } else if (row.value.case === 'metered') {
      const m = row.value.value
      flippedValue = {
        value: {
          case: 'meteredValue' as const,
          value: {
            limit: m.limit,
            resetPeriod: m.resetPeriod,
            overageBehavior: m.overageBehavior,
            warningThresholdPct: m.warningThresholdPct,
            enabled: !m.enabled,
          },
        },
      }
    } else {
      flippedValue = { value: { case: undefined } }
    }

    if (pinnedHere) {
      // We have a local entitlement row — update it
      const local = localEntitlements.find(e => e.featureId === featureId)
      if (local) {
        updateMutation.mutate({
          id: local.id,
          value: new EntitlementValue(flippedValue),
        })
        return
      }
    }
    // No local row yet — create one with the flipped flag
    createMutation.mutate({
      featureId,
      entity: protoEntity,
      value: new EntitlementValue(flippedValue),
    })
  }

  const handleRemove = () => {
    const featureId = row.feature?.id
    if (!featureId) return
    const local = localEntitlements.find(e => e.featureId === featureId)
    if (local) {
      deleteMutation.mutate({ id: local.id })
    }
  }

  // "Remove local override" is only meaningful when we have the local row id
  const localRow = localEntitlements.find(e => e.featureId === row.feature?.id)
  const canRemove = pinnedHere && !!localRow

  const isBusy =
    createMutation.isPending || updateMutation.isPending || deleteMutation.isPending

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className="p-1 hover:bg-muted rounded text-muted-foreground"
          aria-label={`Actions for ${featureName}`}
        >
          <MoreVerticalIcon size={14} />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-[200px]">
        {/* Override */}
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuItem
              onSelect={() => onOverride(row)}
              disabled={isBusy}
            >
              Override
            </DropdownMenuItem>
          </TooltipTrigger>
          <TooltipContent side="left" className="max-w-56">
            {`Edit this entitlement for this ${label}. Saves a local copy with your changes.`}
          </TooltipContent>
        </Tooltip>

        {/* Pin (only when canPin and not already pinned here) */}
        {canPin && !pinnedHere && (
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuItem
                onSelect={handlePin}
                disabled={isBusy || entity.type === 'product'}
              >
                Pin
              </DropdownMenuItem>
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-56">
              {`Save a local copy of this entitlement on this ${label}. Same value as upstream, but locked even if upstream changes.`}
            </TooltipContent>
          </Tooltip>
        )}

        {/* Disable / Enable toggle */}
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuItem
              onSelect={handleToggleDisable}
              disabled={isBusy || entity.type === 'product'}
            >
              {disabled ? 'Enable' : 'Disable'}
            </DropdownMenuItem>
          </TooltipTrigger>
          <TooltipContent side="left" className="max-w-56">
            {disabled
              ? `Re-enable this entitlement on this ${label}.`
              : `Mark this entitlement as disabled here. Stays visible so you can re-enable it later.`}
          </TooltipContent>
        </Tooltip>

        {/* Remove local override */}
        {pinnedHere && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <DropdownMenuItem
                  onSelect={handleRemove}
                  disabled={isBusy || !canRemove}
                  className={cn(!canRemove && 'cursor-not-allowed opacity-50')}
                >
                  Remove local override
                </DropdownMenuItem>
              </span>
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-56">
              {canRemove
                ? `Delete the local copy. The entitlement falls back to the upstream value.`
                : 'Coming soon — local row id not available.'}
            </TooltipContent>
          </Tooltip>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// ── Main panel ────────────────────────────────────────────────────────────────

export const ResolvedEntitlementsPanel = forwardRef<ResolvedEntitlementsPanelHandle, Props>(({
  entity,
  canPin,
  hideAddButton = false,
}, ref) => {
  const label = ENTITY_LABEL[entity.type]
  const { entitlements: resolved, isLoading } = useResolvedEntitlementsForEntity(entity)
  const batchCreate = useBatchCreateEntitlements(entity)

  // Dialog state for Override / Add
  const [dialog, setDialog] = useState<
    | { open: false }
    | { open: true; row?: ResolvedEntitlement }
  >({ open: false })

  // Fetch direct entitlements for this entity so we can look up row ids for deletion.
  // Product has no proto EntitlementEntity variant — skip the query for it.
  const localEntitlementsQuery = useQuery(
    listEntitlementsByEntity,
    entity.type !== 'product'
      ? { entity: toEntitlementEntity(entity) }
      : { entity: undefined as unknown as EntitlementEntity }
  )
  const localEntitlements: Entitlement[] =
    entity.type !== 'product'
      ? (localEntitlementsQuery.data?.entitlements ?? [])
      : []

  const queryClient = useQueryClient()
  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: [listEntitlementsByEntity.service.typeName] })
  }

  // Pin all: confirmation gate + batch-create every entitlement not yet pinned here.
  const unpinnedRows = resolved.filter(r => !isPinnedHere(r, entity))
  const [pinAllOpen, setPinAllOpen] = useState(false)

  const requestPinAll = () => {
    if (entity.type === 'product' || batchCreate.disabled) return
    if (unpinnedRows.length === 0) {
      toast.info('All entitlements are already pinned on this ' + label + '.')
      return
    }
    setPinAllOpen(true)
  }

  const confirmPinAll = () => {
    batchCreate.mutate(
      unpinnedRows.map(r => ({
        featureId: r.feature!.id,
        value: entitlementValueToSpec(r.value),
      }))
    )
    setPinAllOpen(false)
  }

  // On a product detail page every row belongs to that product (or to no product), so
  // grouping by product is redundant — fall back to a single flat list.
  const showGrouping = entity.type !== 'product'
  const groups = showGrouping
    ? groupByProduct(resolved, r =>
        r.feature?.product ? { id: r.feature.product.id, name: r.feature.product.name } : undefined
      )
    : [{ id: null, name: '', items: resolved }]

  // Proto entity for dialog (product has no proto variant — undefined disables the dialog)
  const protoEntity: PartialMessage<EntitlementEntity> | undefined =
    entity.type !== 'product'
      ? toEntitlementEntity(entity)
      : undefined

  // Exclude all features already resolved on this entity (locally pinned OR inherited).
  // Already-resolved features should be edited via the Override action, not added again.
  const existingFeatureIds = new Set<string>([
    ...localEntitlements.map(e => e.featureId),
    ...resolved.map(r => r.feature?.id ?? '').filter(Boolean),
  ])

  // Add-on entitlements are inherited from the linked product; users can only override
  // existing entitlement values, not add new ones. Product surfaces never have an Add.
  const showAddButton =
    entity.type !== 'product' && entity.type !== 'add-on' && !hideAddButton

  useImperativeHandle(ref, () => ({
    openAdd: () => {
      if (entity.type === 'product') return
      setDialog({ open: true })
    },
  }))
  const showPinAll = canPin && entity.type !== 'product'
  const hasHeaderActions = showAddButton || showPinAll

  return (
    <TooltipProvider>
      <div className="flex flex-col gap-4">
        {/* Action row (no internal title — outer page section already provides one) */}
        {hasHeaderActions && (
          <div className="flex items-center justify-end gap-2">
            {/* Add entitlement (hidden on Product context — entitlements live on plan version / add-on) */}
            {showAddButton && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => setDialog({ open: true })}
                  >
                    Add entitlement
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {`Attach a new entitlement to this ${label}.`}
                </TooltipContent>
              </Tooltip>
            )}

            {/* Pin all (only when canPin and entity is not product) */}
            {showPinAll && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={requestPinAll}
                    disabled={batchCreate.isPending}
                  >
                    Pin all
                  </Button>
                </TooltipTrigger>
                <TooltipContent className="max-w-56">
                  {`Save local copies of every upstream entitlement on this ${label}. Already-pinned entries are skipped.`}
                </TooltipContent>
              </Tooltip>
            )}
          </div>
        )}

        {/* Body */}
        {isLoading ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        ) : resolved.length === 0 ? (
          <p className="text-sm text-muted-foreground">No entitlements resolved for this {label}.</p>
        ) : (
          groups.map(group => (
            <div key={group.id ?? '__general__'}>
              {showGrouping && (
                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-1 px-1">
                  {group.name}
                </p>
              )}
              <div className="border border-border rounded-lg divide-y divide-border overflow-hidden">
                {[...group.items]
                  .sort((a, b) => (a.feature?.name ?? '').localeCompare(b.feature?.name ?? ''))
                  .map(r => {
                  const featureId = r.feature?.id ?? ''
                  const featureName = r.feature?.name ?? featureId
                  const disabled = isEntitlementDisabled(r.value)
                  const pinned = isPinnedHere(r, entity)

                  return (
                    <div
                      key={featureId}
                      className={cn(
                        'group flex items-center justify-between px-4 py-2.5 text-sm',
                        disabled && 'opacity-60'
                      )}
                    >
                      {/* Feature name + inheritance icon + disabled badge. */}
                      <div className="flex items-center gap-2 min-w-0">
                        <span className="font-medium truncate">{featureName}</span>
                        {(() => {
                          const featureProduct = r.feature?.product
                          const isDirectlySet =
                            entity.type === 'product' ? !!featureProduct : pinned
                          if (isDirectlySet) return null
                          // On a product surface the row is, by definition, inherited
                          // from a global (cross-product) feature.
                          const tooltip =
                            entity.type === 'product'
                              ? 'Inherited from a global feature.'
                              : buildInheritanceTooltip(r.origin, featureProduct)
                          return <InheritedIcon tooltip={tooltip} />
                        })()}
                        {disabled && (
                          <Badge variant="secondary" className="text-xs shrink-0">
                            Disabled
                          </Badge>
                        )}
                      </div>

                      {/* Value + actions */}
                      <div className="flex items-center gap-2 shrink-0 ml-4">
                        <span className="text-muted-foreground text-xs tabular-nums">
                          {formatResolvedValue(r.value)}
                        </span>
                        <RowActions
                          row={r}
                          entity={entity}
                          label={label}
                          canPin={canPin}
                          pinnedHere={pinned}
                          localEntitlements={localEntitlements}
                          onOverride={row => setDialog({ open: true, row })}
                          onInvalidate={invalidate}
                        />
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          ))
        )}

        {/* Override / Add dialog */}
        {dialog.open && protoEntity && (() => {
          const existing = dialog.row
            ? localEntitlements.find(e => e.featureId === dialog.row?.feature?.id)
            : undefined
          // Inherited row (no local override yet) → seed the dialog with the resolved value
          // so the override form is pre-filled with the current value.
          const seedValue =
            dialog.row && !existing
              ? new EntitlementValue(entitlementValueToSpec(dialog.row.value))
              : undefined
          return (
            <EntityEntitlementDialog
              entity={protoEntity}
              existing={existing}
              seedValue={seedValue}
              featureId={dialog.row?.feature?.id}
              existingFeatureIds={existingFeatureIds}
              onClose={() => setDialog({ open: false })}
            />
          )
        })()}

        {/* Pin all confirmation */}
        <AlertDialog open={pinAllOpen} onOpenChange={setPinAllOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Pin all entitlements?</AlertDialogTitle>
              <AlertDialogDescription>
                {`This will save local copies of ${unpinnedRows.length} inherited ${
                  unpinnedRows.length === 1 ? 'entitlement' : 'entitlements'
                } on this ${label}. Pinned values stay fixed even if upstream changes.`}
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
})

ResolvedEntitlementsPanel.displayName = 'ResolvedEntitlementsPanel'
