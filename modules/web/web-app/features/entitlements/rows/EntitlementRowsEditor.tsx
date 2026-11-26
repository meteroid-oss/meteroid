import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
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
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Skeleton,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { HelpCircle, HistoryIcon, Pencil, PlusIcon, Trash2 } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { type LevelKey, type Source, levelKeyOf, sourceOf } from '@/features/entitlements/model'
import { type GrantAction, GrantsTable } from '@/features/entitlements/rows/GrantsTable'
import { type RowScope, RowSheet } from '@/features/entitlements/rows/RowSheet'
import { GrantValue, GrantWhen, isOverrideSource } from '@/features/entitlements/rows/grantDisplay'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteEntitlement,
  listEntitlementHistory,
  listEntitlementsByEntity,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

import type { Entitlement, Feature } from '@/rpc/api/entitlements/v1/models_pb'

type Props = RowScope & {
  canEdit?: boolean
  /** Per row, when some rows stay editable without `canEdit`: a plan's rows on a published version. */
  canEditRow?: (row: Entitlement) => boolean
  /** Empty when the parent already titles the section: only the add action renders. */
  title?: string
  /** Replaces the title, on the add action's line. */
  heading?: ReactNode
  /** Between the header and the rows. */
  children?: ReactNode
  emptyText?: string
  onEvaluate?: (feature: Feature) => void
  /** A secondary label of limits counted from a metric, by metric id ("Paid with credit"). */
  metricLabels?: Map<string, string>
  /** Fixes the feature of new entitlements, e.g. a feature's own defaults. */
  feature?: Feature
}

type Editing = { existing?: Entitlement; feature?: Feature } | undefined

const levelRank = (k: LevelKey) => (k === 'feature' ? 0 : k === 'customer' ? 1 : 2)

export const EntitlementRowsEditor = ({
  canEdit = true,
  canEditRow,
  title = 'Entitlements',
  heading,
  children,
  emptyText = 'No entitlements',
  onEvaluate,
  metricLabels,
  feature: fixedFeature,
  ...scope
}: Props) => {
  const [editing, setEditing] = useState<Editing>()
  const [history, setHistory] = useState<Entitlement>()
  const [deleting, setDeleting] = useState<Entitlement>()
  const source = sourceOf(scope.owner, scope.resource?.id)

  const query = useQuery(listEntitlementsByEntity, { entity: { EntityId: scope.owner } })
  const features = useMemo(() => new Map((query.data?.features ?? []).map(f => [f.id, f])), [query.data])
  const rows = useMemo(() => {
    const name = (e: Entitlement) => features.get(e.featureId)?.name ?? e.featureId
    return (query.data?.entitlements ?? [])
      .filter(e => e.resourceId === scope.resource?.id)
      .sort(
        (a, b) =>
          name(a).localeCompare(name(b)) ||
          a.featureId.localeCompare(b.featureId) ||
          levelRank(levelKeyOf(a)) - levelRank(levelKeyOf(b)) ||
          levelKeyOf(a).localeCompare(levelKeyOf(b)) ||
          a.createdAt.localeCompare(b.createdAt)
      )
  }, [query.data, features, scope.resource?.id])

  const queryClient = useQueryClient()
  const deleteMutation = useMutation(deleteEntitlement, {
    onSuccess: () => {
      setDeleting(undefined)
      return queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listEntitlementsByEntity.parent, cardinality: undefined }),
      })
    },
    onError: err => toast.error(err.message),
  })

  const allVersions = (e: Entitlement) =>
    scope.owner.case === 'planVersionId' && e.entity?.EntityId.case === 'planId' ? (
      <Badge variant="secondary" size="sm" className="font-normal">
        All versions
      </Badge>
    ) : undefined

  return (
    <section className="flex flex-col gap-3">
      {(heading || title || canEdit) && (
        <div className={cn('flex items-center gap-2', heading || title ? 'justify-between' : 'justify-end')}>
          {heading ?? (title && <h3 className="text-sm font-semibold">{title}</h3>)}
          {canEdit && (
            <Button size="sm" variant="outline" hasIcon onClick={() => setEditing({ feature: fixedFeature })}>
              <PlusIcon size={14} />{' '}
              {source === 'default'
                ? 'Add default'
                : isOverrideSource(source)
                  ? 'Add override'
                  : 'Add entitlement'}
            </Button>
          )}
        </div>
      )}
      {children}

      {query.isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : rows.length === 0 ? (
        <p className="rounded-lg border border-border px-4 py-6 text-center text-sm text-muted-foreground">
          {emptyText}
        </p>
      ) : (
        <GrantsTable
          rows={rows.map(e => {
            const feature = features.get(e.featureId)
            const metricLabel = feature?.limit?.metricId ? metricLabels?.get(feature.limit.metricId) : undefined
            const editable = canEditRow ? canEditRow(e) : canEdit
            const actions: GrantAction[] = [
              ...(editable && feature
                ? [{ label: 'Edit', icon: Pencil, onSelect: () => setEditing({ existing: e, feature }) }]
                : []),
              { label: 'History', icon: HistoryIcon, onSelect: () => setHistory(e) },
              ...(onEvaluate && feature
                ? [{ label: 'Why this value…', icon: HelpCircle, onSelect: () => onEvaluate(feature) }]
                : []),
              ...(editable
                ? [{ label: 'Remove', icon: Trash2, destructive: true, onSelect: () => setDeleting(e) }]
                : []),
            ]
            return {
              key: e.id,
              featureId: e.featureId,
              feature,
              value: <GrantValue row={e} feature={feature} source={source} metricLabel={metricLabel} />,
              when: <GrantWhen row={e} badges={allVersions(e)} />,
              actions,
            }
          })}
        />
      )}

      {editing && <RowSheet {...scope} {...editing} onClose={() => setEditing(undefined)} />}
      {history && (
        <HistoryDialog
          row={history}
          feature={features.get(history.featureId)}
          source={source}
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
    </section>
  )
}

const actorLabel = (type: string) => {
  if (type === 'API_KEY') return 'API key'
  const words = type.toLowerCase().replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** Every revision: an edit revokes the open row and inserts its successor. */
export const HistoryDialog = ({
  row,
  feature,
  source,
  onClose,
}: {
  row: Entitlement
  feature?: Feature
  source: Source
  onClose: () => void
}) => {
  const query = useQuery(listEntitlementHistory, row.id ? { id: row.id } : skipToken)
  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>History · {feature?.name ?? row.featureId}</DialogTitle>
        </DialogHeader>
        {query.isLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : (
          <ol className="divide-y divide-border rounded-lg border border-border">
            {(query.data?.entitlements ?? []).map(revision => (
              <li
                key={`${revision.updatedAt}-${revision.revokedAt ?? 'open'}`}
                className="flex items-start justify-between gap-4 p-3"
              >
                <div className="flex min-w-0 flex-col gap-1.5">
                  <GrantValue row={revision} feature={feature} source={source} />
                  <GrantWhen row={revision} />
                  {revision.reason && <span className="text-xs text-muted-foreground">{revision.reason}</span>}
                </div>
                <div className="flex shrink-0 flex-col items-end gap-1 text-xs text-muted-foreground">
                  <Badge variant={revision.revokedAt ? 'outline' : 'secondary'} size="sm">
                    {revision.revokedAt ? 'Replaced' : 'Current'}
                  </Badge>
                  <span>{parseAndFormatDateTime(revision.updatedAt)}</span>
                  {revision.createdBy && (
                    <span title={revision.createdBy.id}>{actorLabel(revision.createdBy.type)}</span>
                  )}
                </div>
              </li>
            ))}
          </ol>
        )}
      </DialogContent>
    </Dialog>
  )
}
