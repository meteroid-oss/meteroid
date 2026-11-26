import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, HistoryIcon, Pencil, PlusIcon, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { HistoryDialog } from '@/features/entitlements/rows/EntitlementRowsEditor'
import { type GrantAction, GrantsTable, type GrantsTableRow } from '@/features/entitlements/rows/GrantsTable'
import { RowSheet } from '@/features/entitlements/rows/RowSheet'
import { GrantValue, grantValueText, plainValue } from '@/features/entitlements/rows/grantDisplay'
import { inheritedValue } from '@/features/entitlements/rows/grantDraft'
import { useFeatures } from '@/features/entitlements/rows/useFeatures'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteEntitlement,
  getResolvedEntitlementsForSubscription,
  listEntitlementsByEntity,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { EntitlementValueSchema } from '@/rpc/api/entitlements/v1/models_pb'

import type { Entitlement, Feature, ResolvedEntitlement, ResolvedOrigin } from '@/rpc/api/entitlements/v1/models_pb'

type Props = { subscriptionId: string; canEdit: boolean }
type Editing = { existing?: Entitlement; feature?: Feature }

const named = (kind: string, name: string | undefined) => (name ? `${kind} · ${name}` : kind)

function fromLabel(origin: ResolvedOrigin | undefined, subscriptionId: string): string {
  const entity = origin?.entity?.EntityId
  switch (entity?.case) {
    case 'featureId':
      return 'Default'
    case 'planId':
      return named('Plan', origin?.name)
    case 'planVersionId':
      return named('Plan version', origin?.name)
    case 'addOnId':
      return named('Add-on', origin?.name)
    case 'subscriptionId':
      return entity.value === subscriptionId ? 'Override' : named('Subscription', origin?.name)
    case 'quoteId':
      return named('Quote', origin?.name)
    case 'customerId':
      return 'Customer override'
    default:
      return '—'
  }
}

/** What the subscription gives, whoever decides it, with its own overrides editable. */
export const SubscriptionEntitlementsCard = ({ subscriptionId, canEdit }: Props) => {
  const owner = { case: 'subscriptionId', value: subscriptionId } as const
  const [editing, setEditing] = useState<Editing>()
  const [history, setHistory] = useState<Entitlement>()
  const [deleting, setDeleting] = useState<Entitlement>()

  const resolvedQuery = useQuery(getResolvedEntitlementsForSubscription, { subscriptionId })
  const ownQuery = useQuery(listEntitlementsByEntity, { entity: { EntityId: owner } })
  const resolved = (resolvedQuery.data?.entitlements ?? []).filter(r => r.feature?.id)
  const own = (ownQuery.data?.entitlements ?? []).filter(
    e => e.entity?.EntityId.case === 'subscriptionId' && !e.resourceId
  )
  const features = useFeatures([...new Set([...resolved.map(r => r.feature!.id), ...own.map(e => e.featureId)])])
  const featureOf = (id: string) =>
    features.get(id) ?? ownQuery.data?.features.find(f => f.id === id)

  // Every service query, the resolved list included.
  const queryClient = useQueryClient()
  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      setDeleting(undefined)
      return queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: deleteEntitlement.parent, cardinality: undefined }),
      })
    },
    onError: err => toast.error(err.message),
  })

  const ownActions = (rows: Entitlement[]): GrantAction[] =>
    rows.flatMap(e => {
      const feature = featureOf(e.featureId)
      const suffix = rows.length > 1 ? ` · ${grantValueText(e, feature, 'subscription')}` : ''
      return [
        ...(canEdit && feature
          ? [{ label: `Edit${suffix}`, icon: Pencil, onSelect: () => setEditing({ existing: e, feature }) }]
          : []),
        { label: `History${suffix}`, icon: HistoryIcon, onSelect: () => setHistory(e) },
        ...(canEdit
          ? [{ label: `Remove${suffix}`, icon: Trash2, destructive: true, onSelect: () => setDeleting(e) }]
          : []),
      ]
    })

  const resolvedRow = (r: ResolvedEntitlement): GrantsTableRow => {
    const id = r.feature!.id
    const feature = featureOf(id)
    const rows = own.filter(e => e.featureId === id)
    const decidedBy = r.ownGrant && (r.ownGrant.decidingPlanName ?? 'another subscription')
    return {
      key: id,
      featureId: id,
      feature,
      featureName: r.feature!.name,
      product: r.feature!.product,
      value: (
        <span className="text-sm font-medium">
          {plainValue(create(EntitlementValueSchema, inheritedValue(r)), feature)}
        </span>
      ),
      when: (
        <div className="flex flex-col gap-0.5">
          <span className="text-sm">{fromLabel(r.origin, subscriptionId)}</span>
          {decidedBy && <span className="text-xs text-muted-foreground">Decided by {decidedBy}</span>}
        </div>
      ),
      actions:
        rows.length > 0
          ? ownActions(rows)
          : canEdit && feature
            ? [{ label: 'Override…', icon: Pencil, onSelect: () => setEditing({ feature }) }]
            : [],
    }
  }

  // Own rows that decide nothing now (not yet started, ended or conditional) stay reachable.
  const idle = own.filter(e => !resolved.some(r => r.feature!.id === e.featureId))
  const idleRows: GrantsTableRow[] = [...new Set(idle.map(e => e.featureId))].map(id => {
    const rows = idle.filter(e => e.featureId === id)
    const feature = featureOf(id)
    return {
      key: `own-${id}`,
      featureId: id,
      feature,
      value: <GrantValue row={rows[0]} feature={feature} source="subscription" />,
      when: (
        <div className="flex flex-col gap-0.5">
          <span className="text-sm">Override</span>
          <span className="text-xs text-muted-foreground">Not in effect</span>
        </div>
      ),
      actions: ownActions(rows),
    }
  })

  const loading = resolvedQuery.isLoading || ownQuery.isLoading
  const count = resolved.length
  if (!loading && count === 0 && idleRows.length === 0 && !canEdit) return null

  return (
    <Collapsible className="mb-6 rounded-lg border border-border bg-card shadow-sm">
      <div className="flex items-center gap-2 pr-4">
        <CollapsibleTrigger className="group flex flex-1 items-center justify-between p-4 text-left">
          <h3 className="text-md font-medium text-foreground">
            Entitlements · {count} {count === 1 ? 'feature' : 'features'}
          </h3>
          <ChevronDown className="h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
        </CollapsibleTrigger>
        {canEdit && (
          <Button size="sm" variant="outline" hasIcon onClick={() => setEditing({})}>
            <PlusIcon size={14} /> Add override
          </Button>
        )}
      </div>
      <CollapsibleContent className="border-t border-border p-4">
        {loading ? (
          <Skeleton className="h-24 w-full" />
        ) : count + idleRows.length === 0 ? (
          <p className="rounded-lg border border-border px-4 py-6 text-center text-sm text-muted-foreground">
            No entitlements
          </p>
        ) : (
          <GrantsTable
            whenLabel="From"
            rows={[...resolved.map(resolvedRow), ...idleRows].sort((a, b) =>
              (a.feature?.name ?? a.featureName ?? '').localeCompare(b.feature?.name ?? b.featureName ?? '')
            )}
          />
        )}
      </CollapsibleContent>

      {editing && <RowSheet owner={owner} {...editing} onClose={() => setEditing(undefined)} />}
      {history && (
        <HistoryDialog
          row={history}
          feature={featureOf(history.featureId)}
          source="subscription"
          onClose={() => setHistory(undefined)}
        />
      )}
      <AlertDialog open={!!deleting} onOpenChange={open => !open && setDeleting(undefined)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this entitlement?</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={deleteMutation.isPending}
              onClick={() => deleting && deleteMutation.mutate({ id: deleting.id })}
            >
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Collapsible>
  )
}
