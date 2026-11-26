import { create } from '@bufbuild/protobuf'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Skeleton,
  cn,
} from '@md/ui'
import { Pencil, Pin, PinOff, PlusIcon, Power, PowerOff, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  type SelectionInput,
  useResolvedEntitlementsForEntity,
  useResolvedEntitlementsForSelection,
} from '@/features/entitlements/resolved/useResolvedEntitlements'
import { type GrantAction, GrantsTable, type GrantsTableRow } from '@/features/entitlements/rows/GrantsTable'
import { type DraftScope, RowSheet } from '@/features/entitlements/rows/RowSheet'
import { GrantValue, GrantWhen } from '@/features/entitlements/rows/grantDisplay'
import {
  type GrantDraft,
  draftEffect,
  draftRow,
  hasSwitch,
  inheritedDraft,
  inheritedValue,
  isEnabled,
  withEnabled,
} from '@/features/entitlements/rows/grantDraft'
import { useFeatures } from '@/features/entitlements/rows/useFeatures'
import { originLabel } from '@/features/entitlements/utils'
import { EntitlementSchema } from '@/rpc/api/entitlements/v1/models_pb'

import type { Feature, ResolvedEntitlement } from '@/rpc/api/entitlements/v1/models_pb'

type Props = DraftScope & {
  drafts: GrantDraft[]
  onChange: (drafts: GrantDraft[]) => void
  /** What the new owner inherits, listed with Override… and Pin. */
  inherited: { entitlements: ResolvedEntitlement[]; isLoading: boolean }
  /**
   * Changes on a subscription or quote: drafts can be added and switched off, Pin all asks first,
   * and an edited draft moves last.
   */
  pending?: boolean
  title?: string
  emptyText?: string
}

type Row = { featureId: string; name: string; inherited?: ResolvedEntitlement; draft?: GrantDraft }
type Editing = { feature?: Feature; existing?: GrantDraft }

const keyOf = (d: GrantDraft) => d.spec.featureId

const inheritedOn = (r: ResolvedEntitlement) =>
  r.value.case === 'boolean' || r.value.case === 'metered' ? r.value.value.enabled : true

/** Entitlements collected before their owner exists, over what it inherits; submitted with it. */
const DraftGrantsEditor = ({
  drafts,
  onChange,
  inherited,
  pending = false,
  title = '',
  emptyText = 'No entitlements',
  ...scope
}: Props) => {
  const [editing, setEditing] = useState<Editing>()
  const [confirmingPinAll, setConfirmingPinAll] = useState(false)
  const multi = !!scope.multiInstance

  const inheritedRows = inherited.entitlements.filter(r => r.feature?.id)
  const ids = [...new Set([...inheritedRows.map(r => r.feature!.id), ...drafts.map(keyOf)])]
  const features = useFeatures(ids)
  const draftsById = new Map(drafts.map(d => [keyOf(d), d]))

  const rows: Row[] = [
    ...inheritedRows.map(r => ({
      featureId: r.feature!.id,
      name: r.feature!.name,
      inherited: r,
      draft: draftsById.get(r.feature!.id),
    })),
    ...drafts
      .filter(d => !inheritedRows.some(r => r.feature!.id === keyOf(d)))
      .map(d => ({ featureId: keyOf(d), name: d.feature.name, draft: d })),
  ].sort((a, b) => a.name.localeCompare(b.name))

  const upsert = (draft: GrantDraft) => {
    const i = drafts.findIndex(d => keyOf(d) === keyOf(draft))
    if (pending || i < 0) onChange([...drafts.filter(d => keyOf(d) !== keyOf(draft)), draft])
    else onChange(drafts.map((d, j) => (j === i ? draft : d)))
  }
  const remove = (featureId: string) => onChange(drafts.filter(d => keyOf(d) !== featureId))
  const pin = (r: ResolvedEntitlement) => upsert(inheritedDraft(r, features.get(r.feature!.id)))

  const unpinned = inheritedRows.filter(r => !draftsById.has(r.feature!.id))
  const pinAll = () =>
    onChange([...drafts, ...unpinned.map(r => inheritedDraft(r, features.get(r.feature!.id)))])
  const requestPinAll = () => {
    if (unpinned.length === 0) toast.info('All entitlements are already pinned.')
    else if (pending) setConfirmingPinAll(true)
    else pinAll()
  }

  const switchedOn = (row: Row) =>
    row.draft ? isEnabled(row.draft.spec.value) : row.inherited ? inheritedOn(row.inherited) : true
  const toggle = (row: Row) => {
    const base = row.draft ?? (row.inherited && inheritedDraft(row.inherited, features.get(row.featureId)))
    if (base) upsert(withEnabled(base, !switchedOn(row)))
  }

  const actionsOf = (row: Row): GrantAction[] => {
    const feature = features.get(row.featureId) ?? row.draft?.feature
    const isBoolean = row.draft
      ? row.draft.spec.value?.value.case === 'booleanValue'
      : row.inherited?.value.case === 'boolean'
    const switchable = row.draft
      ? hasSwitch(row.draft.spec.value)
      : row.inherited?.value.case === 'boolean' || row.inherited?.value.case === 'metered'
    const open = () =>
      feature &&
      setEditing({
        feature,
        existing: row.draft ?? (row.inherited && inheritedDraft(row.inherited, feature)),
      })
    const actions: GrantAction[] = []
    if (feature && !(pending && isBoolean))
      actions.push({ label: row.draft ? 'Edit…' : 'Override…', icon: Pencil, onSelect: open })
    if (row.draft && row.inherited)
      actions.push({ label: 'Unpin', icon: PinOff, onSelect: () => remove(row.featureId) })
    else if (row.inherited) {
      const r = row.inherited
      actions.push({ label: 'Pin', icon: Pin, onSelect: () => pin(r) })
    }
    if (pending && switchable)
      actions.push(
        switchedOn(row)
          ? { label: 'Disable', icon: PowerOff, onSelect: () => toggle(row) }
          : { label: 'Enable', icon: Power, onSelect: () => toggle(row) }
      )
    if (row.draft && !row.inherited)
      actions.push({ label: 'Remove', icon: Trash2, destructive: true, onSelect: () => remove(row.featureId) })
    return actions
  }

  const tableRow = (row: Row): GrantsTableRow => {
    const feature = features.get(row.featureId) ?? row.draft?.feature
    if (row.draft) {
      const shown = draftRow(row.draft, draftEffect(row.draft, scope.source, multi))
      return {
        key: row.featureId,
        featureId: row.featureId,
        feature,
        featureName: row.name,
        value: <GrantValue row={shown} feature={feature} source={scope.source} />,
        when: (
          <GrantWhen
            row={shown}
            badges={
              <Badge variant="secondary" size="sm" className="font-normal">
                Pending
              </Badge>
            }
          />
        ),
        actions: actionsOf(row),
      }
    }
    const r = row.inherited!
    const shown = create(EntitlementSchema, { featureId: row.featureId, value: inheritedValue(r) })
    return {
      key: row.featureId,
      featureId: row.featureId,
      feature,
      featureName: row.name,
      value: <GrantValue row={shown} feature={feature} source="plan" />,
      when: (
        <GrantWhen
          row={shown}
          badges={
            r.origin && (
              <Badge variant="outline" size="sm" className="font-normal">
                {originLabel(r.origin)}
              </Badge>
            )
          }
        />
      ),
      actions: actionsOf(row),
    }
  }

  const canAdd = pending
  const canPinAll = inheritedRows.length > 0

  return (
    <section className="flex flex-col gap-3">
      {(title || canAdd || canPinAll) && (
        <div className={cn('flex items-center gap-2', title ? 'justify-between' : 'justify-end')}>
          {title && <h3 className="text-sm font-semibold">{title}</h3>}
          <div className="flex items-center gap-2">
            {canPinAll && (
              <Button type="button" size="sm" variant="outline" hasIcon onClick={requestPinAll}>
                <Pin size={14} /> Pin all
              </Button>
            )}
            {canAdd && (
              <Button type="button" size="sm" variant="outline" hasIcon onClick={() => setEditing({})}>
                <PlusIcon size={14} /> Add entitlement
              </Button>
            )}
          </div>
        </div>
      )}

      {inherited.isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : rows.length === 0 ? (
        <p className="rounded-lg border border-border px-4 py-6 text-center text-sm text-muted-foreground">
          {emptyText}
        </p>
      ) : (
        <GrantsTable rows={rows.map(tableRow)} />
      )}

      {editing && (
        <RowSheet
          {...scope}
          {...editing}
          excludeFeatureIds={new Set(drafts.map(keyOf))}
          onDraft={upsert}
          onClose={() => setEditing(undefined)}
        />
      )}

      <AlertDialog open={confirmingPinAll} onOpenChange={setConfirmingPinAll}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Pin all entitlements?</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                pinAll()
                setConfirmingPinAll(false)
              }}
            >
              Pin all
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}

type InheritingProps = Omit<Props, 'source' | 'inherited' | 'pending'>

/** A new subscription's or quote's changes over its plan and add-ons. */
export const SelectionGrantsEditor = ({ selection, ...props }: InheritingProps & { selection: SelectionInput }) => {
  const { entitlements, isLoading } = useResolvedEntitlementsForSelection(selection)
  return <DraftGrantsEditor {...props} source="subscription" pending inherited={{ entitlements, isLoading }} />
}

/** A new add-on's entitlements over its product's. */
export const ProductGrantsEditor = ({ productId, ...props }: InheritingProps & { productId: string }) => {
  const { entitlements, isLoading } = useResolvedEntitlementsForEntity({ type: 'product', id: productId })
  return <DraftGrantsEditor {...props} source="addOn" inherited={{ entitlements, isLoading }} />
}
