/**
 * PendingEntitlementsPanel — entitlements panel for an entity that does not yet exist
 * (currently used during quote creation). Shows the upstream resolved entitlements
 * (inherited from a plan version) layered with the user's pending in-form specs.
 * All state is local; the parent form submits the specs at creation time.
 */
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
import { cn } from '@ui/lib'
import { GitMerge, MoreVerticalIcon } from 'lucide-react'
import { FC, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { InheritedIcon } from '@/features/entitlements/InheritedIcon'
import { EntitlementSpecDialog } from '@/features/entitlements/creation/EntitlementSpecDialog'
import {
  PendingEntitlementSpec,
  pendingSpecKey,
  resolvedToPendingSpec,
} from '@/features/entitlements/creation/types'
import {
  useResolvedEntitlementsForSelection,
  type SelectionInput,
} from '@/features/entitlements/resolved/useResolvedEntitlements'
import {
  buildInheritanceTooltip,
  formatResolvedValue,
  groupByProduct,
  isEntitlementDisabled,
} from '@/features/entitlements/utils'
import { ResolvedEntitlement } from '@/rpc/api/entitlements/v1/models_pb'

// ── Types ─────────────────────────────────────────────────────────────────────

type MergedRow = {
  featureId: string
  featureName: string
  productId?: string
  productName?: string
  /** Display label for the current winning value */
  valueLabel: string
  /** Whether the feature is disabled in the winning value */
  disabled: boolean
  /** Set when the user has a local pending spec for this feature */
  pending?: PendingEntitlementSpec
  /** Set when the plan version has an inherited entitlement for this feature */
  inherited?: ResolvedEntitlement
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function isPendingDisabled(spec: PendingEntitlementSpec): boolean {
  if (spec.featureType === 'boolean') return spec.boolEnabled === false
  if (spec.featureType === 'metered') return spec.meteredEnabled === false
  return false
}

function formatPendingValue(spec: PendingEntitlementSpec): string {
  if (spec.featureType === 'boolean') {
    return spec.boolEnabled !== false ? 'Enabled' : 'Disabled'
  }
  // metered
  const limit = spec.limit ?? '∞'
  const warn = spec.warningThresholdPct != null ? ` (warn @ ${spec.warningThresholdPct}%)` : ''
  return `${limit}${warn}`
}

function buildMergedRows(
  inherited: ResolvedEntitlement[],
  pending: PendingEntitlementSpec[]
): MergedRow[] {
  const rows = new Map<string, MergedRow>()

  // First pass: inherited baseline
  for (const r of inherited) {
    const featureId = r.feature?.id ?? ''
    if (!featureId) continue
    rows.set(featureId, {
      featureId,
      featureName: r.feature?.name ?? featureId,
      productId: r.feature?.product?.id,
      productName: r.feature?.product?.name,
      valueLabel: formatResolvedValue(r.value),
      disabled: isEntitlementDisabled(r.value),
      inherited: r,
    })
  }

  // Second pass: overlay pending specs (replace value label + disabled flag)
  for (const spec of pending) {
    const featureId = pendingSpecKey(spec)
    if (!featureId) {
      if (process.env.NODE_ENV !== 'production') {
        console.warn('Skipping pending entitlement spec without featureId or featureName', spec)
      }
      continue
    }
    const existing = rows.get(featureId)
    rows.set(featureId, {
      featureId,
      featureName: spec.featureDisplayName,
      productId: spec.productId ?? existing?.productId,
      productName: spec.productName ?? existing?.productName,
      valueLabel: formatPendingValue(spec),
      disabled: isPendingDisabled(spec),
      pending: spec,
      inherited: existing?.inherited,
    })
  }

  return Array.from(rows.values())
}

// ── RowActions ────────────────────────────────────────────────────────────────

type RowActionsProps = {
  row: MergedRow
  onOverride: (row: MergedRow) => void
  onPin: (row: MergedRow) => void
  onToggleDisable: (row: MergedRow) => void
  onRemovePending: (featureId: string) => void
}

const RowActions: FC<RowActionsProps> = ({
  row,
  onOverride,
  onPin,
  onToggleDisable,
  onRemovePending,
}) => {
  const hasPending = !!row.pending
  const hasInherited = !!row.inherited
  const featureName = row.featureName

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
        {/* Override — opens dialog pre-filled with current value */}
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuItem onSelect={() => onOverride(row)}>
              Override
            </DropdownMenuItem>
          </TooltipTrigger>
          <TooltipContent side="left" className="max-w-56">
            Edit this entitlement for this quote. Saves a local copy with your changes.
          </TooltipContent>
        </Tooltip>

        {/* Pin — copies the inherited value verbatim into a pending spec (only when inherited exists and not already pinned) */}
        {hasInherited && !hasPending && (
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuItem onSelect={() => onPin(row)}>
                Pin
              </DropdownMenuItem>
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-56">
              Save a local copy of this entitlement on this quote. Same value as the plan version,
              but locked even if the plan changes.
            </TooltipContent>
          </Tooltip>
        )}

        {/* Disable / Enable */}
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuItem onSelect={() => onToggleDisable(row)}>
              {row.disabled ? 'Enable' : 'Disable'}
            </DropdownMenuItem>
          </TooltipTrigger>
          <TooltipContent side="left" className="max-w-56">
            {row.disabled
              ? 'Re-enable this entitlement on this quote.'
              : 'Mark this entitlement as disabled here. Stays visible so you can re-enable it later.'}
          </TooltipContent>
        </Tooltip>

        {/* Remove local override — only when a pending spec exists */}
        {hasPending && (
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuItem
                onSelect={() => onRemovePending(row.featureId)}
                className={cn(!hasInherited && 'text-destructive focus:text-destructive')}
              >
                Remove local override
              </DropdownMenuItem>
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-56">
              {hasInherited
                ? 'Delete the local copy. The entitlement falls back to the plan or add-on value.'
                : 'Remove this entitlement from the quote entirely.'}
            </TooltipContent>
          </Tooltip>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// ── Props ─────────────────────────────────────────────────────────────────────

type Props = {
  selection: SelectionInput
  pending: PendingEntitlementSpec[]
  onChange: (next: PendingEntitlementSpec[]) => void
}

// ── Main panel ────────────────────────────────────────────────────────────────

export const PendingEntitlementsPanel: FC<Props> = ({ selection, pending, onChange }) => {
  // 1. Fetch resolved entitlements for the in-flight selection (plan + add-ons) as the baseline
  const { entitlements: inherited, isLoading } = useResolvedEntitlementsForSelection(selection)

  // 2. Build merged view: pending specs layer on top of inherited rows
  const rows = useMemo(() => buildMergedRows(inherited, pending), [inherited, pending])

  // 3. Group by product (same helper as elsewhere)
  const groups = useMemo(
    () =>
      groupByProduct(rows, r =>
        r.productId ? { id: r.productId, name: r.productName ?? r.productId } : undefined
      ),
    [rows]
  )

  // 4. Dialog state (open/closed + optional pre-fill row)
  const [dialog, setDialog] = useState<
    | { open: false }
    | { open: true; row?: MergedRow }
  >({ open: false })

  // 5. Pin-all confirmation
  const unpinnedRows = rows.filter(r => !r.pending && r.inherited)
  const [pinAllOpen, setPinAllOpen] = useState(false)

  // ── Action handlers ──────────────────────────────────────────────────────

  const upsertPending = (spec: PendingEntitlementSpec) => {
    const featureId = pendingSpecKey(spec)
    if (!featureId) return
    onChange([...pending.filter(p => pendingSpecKey(p) !== featureId), spec])
  }

  const removePending = (featureId: string) => {
    onChange(pending.filter(p => pendingSpecKey(p) !== featureId))
  }

  const handleOverride = (row: MergedRow) => {
    setDialog({ open: true, row })
  }

  const handlePin = (row: MergedRow) => {
    if (!row.inherited) return
    const spec = resolvedToPendingSpec(row.inherited)
    upsertPending(spec)
    toast.success(`"${row.featureName}" pinned on this quote.`)
  }

  const handleToggleDisable = (row: MergedRow) => {
    if (row.pending) {
      // Flip the enabled flag on the existing pending spec
      const spec = row.pending
      if (spec.featureType === 'boolean') {
        upsertPending({ ...spec, boolEnabled: !(spec.boolEnabled !== false) })
      } else {
        upsertPending({ ...spec, meteredEnabled: !spec.meteredEnabled })
      }
    } else if (row.inherited) {
      // No pending spec yet — derive one from inherited and flip enabled
      const newEnabled = row.disabled // currently disabled → we want to enable
      if (row.inherited.value.case === 'boolean') {
        upsertPending(resolvedToPendingSpec(row.inherited, { boolEnabled: newEnabled }))
      } else {
        upsertPending(resolvedToPendingSpec(row.inherited, { meteredEnabled: newEnabled }))
      }
    }
  }

  const requestPinAll = () => {
    if (unpinnedRows.length === 0) {
      toast.info('All entitlements are already pinned on this quote.')
      return
    }
    setPinAllOpen(true)
  }

  const confirmPinAll = () => {
    const newSpecs = unpinnedRows
      .filter(r => r.inherited)
      .map(r => resolvedToPendingSpec(r.inherited!))
    const newKeys = new Set(newSpecs.map(pendingSpecKey).filter((k): k is string => !!k))
    onChange([
      ...pending.filter(p => {
        const k = pendingSpecKey(p)
        return k === null || !newKeys.has(k)
      }),
      ...newSpecs,
    ])
    setPinAllOpen(false)
    toast.success(`${newSpecs.length} entitlement${newSpecs.length === 1 ? '' : 's'} pinned on this quote.`)
  }

  // Build the initialSpec for the Override dialog from the current row state
  const dialogInitialSpec: PendingEntitlementSpec | undefined = useMemo(() => {
    if (!dialog.open || !dialog.row) return undefined
    const row = dialog.row
    // Use existing pending spec if available; otherwise derive from inherited
    if (row.pending) return row.pending
    if (row.inherited) return resolvedToPendingSpec(row.inherited)
    return undefined
  }, [dialog])

  const handleDialogSave = (spec: PendingEntitlementSpec) => {
    upsertPending(spec)
    setDialog({ open: false })
  }

  // ── Render ───────────────────────────────────────────────────────────────

  return (
    <TooltipProvider>
      <div className="flex flex-col gap-4">
        {/* Action row (no internal title — outer card provides one) */}
        <div className="flex items-center justify-end gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="sm"
                  variant="secondary"
                  type="button"
                  onClick={() => setDialog({ open: true })}
                >
                  Add entitlement
                </Button>
              </TooltipTrigger>
              <TooltipContent>Attach a new entitlement to this quote.</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="sm"
                  variant="outline"
                  type="button"
                  onClick={requestPinAll}
                >
                  Pin all
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-56">
                Save local copies of every upstream entitlement on this quote.
                Already-pinned entries are skipped.
              </TooltipContent>
            </Tooltip>
        </div>

        {/* Body */}
        {isLoading ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        ) : rows.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No entitlements on the selected plan and add-ons. Use &ldquo;Add entitlement&rdquo; to attach one.
          </p>
        ) : (
          groups.map(group => (
            <div key={group.id ?? '__general__'}>
              <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-1 px-1">
                {group.name}
              </p>
              <div className="border border-border rounded-lg divide-y divide-border overflow-hidden">
                {[...group.items]
                  .sort((a, b) => a.featureName.localeCompare(b.featureName))
                  .map(row => {
                    const hasPending = !!row.pending
                    const hasInherited = !!row.inherited

                    // Show the inheritance icon only when the row has no local pending
                    // override. The tooltip falls back to a quote-specific phrase when
                    // origin info is missing.
                    const showInheritanceIcon = !hasPending && hasInherited
                    const inheritanceTooltip = hasInherited
                      ? row.inherited!.origin
                        ? buildInheritanceTooltip(
                            row.inherited!.origin,
                            row.inherited!.feature?.product
                          )
                        : 'Inherited from the selected plan and add-ons.'
                      : ''

                    // When pending+inherited: show a "pinned here" icon with a different tooltip
                    const showPinnedIcon = hasPending && hasInherited

                    return (
                      <div
                        key={row.featureId}
                        className={cn(
                          'group flex items-center justify-between px-4 py-2.5 text-sm',
                          row.disabled && 'opacity-60'
                        )}
                      >
                        {/* Left: feature name + icon + disabled badge */}
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="font-medium truncate">{row.featureName}</span>

                          {showPinnedIcon && (
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <span
                                  className="text-primary cursor-help shrink-0"
                                  aria-label="Overridden locally"
                                >
                                  <GitMerge size={14} />
                                </span>
                              </TooltipTrigger>
                              <TooltipContent>
                                Set locally on this quote; overrides plan and add-ons.
                              </TooltipContent>
                            </Tooltip>
                          )}

                          {showInheritanceIcon && <InheritedIcon tooltip={inheritanceTooltip} />}

                          {row.disabled && (
                            <Badge variant="secondary" className="text-xs shrink-0">
                              Disabled
                            </Badge>
                          )}
                        </div>

                        {/* Right: value + kebab menu */}
                        <div className="flex items-center gap-2 shrink-0 ml-4">
                          <span className="text-muted-foreground text-xs tabular-nums">
                            {row.valueLabel}
                          </span>
                          <RowActions
                            row={row}
                            onOverride={handleOverride}
                            onPin={handlePin}
                            onToggleDisable={handleToggleDisable}
                            onRemovePending={removePending}
                          />
                        </div>
                      </div>
                    )
                  })}
              </div>
            </div>
          ))
        )}

        {/* Add / Override dialog */}
        <EntitlementSpecDialog
          open={dialog.open}
          onOpenChange={open => !open && setDialog({ open: false })}
          initialSpec={dialogInitialSpec}
          onAdd={handleDialogSave}
          existingEntitlements={
            dialog.open && dialog.row
              ? // When overriding, exclude the current feature from the "already exists" set
                pending.filter(p => pendingSpecKey(p) !== dialog.row!.featureId)
              : pending
          }
        />

        {/* Pin all confirmation */}
        <AlertDialog open={pinAllOpen} onOpenChange={setPinAllOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Pin all entitlements?</AlertDialogTitle>
              <AlertDialogDescription>
                {`This will save local copies of ${unpinnedRows.length} inherited ${
                  unpinnedRows.length === 1 ? 'entitlement' : 'entitlements'
                } on this quote. Pinned values stay fixed even if the plan version changes.`}
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
