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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import {
  getFeature,
  getFeatureCodeHolder,
  listEntitlementsByEntity,
  listFeatures,
  setFeatureStatus,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { Feature, FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

export type PendingFeatureAction = { feature: Feature; target: FeatureStatus }

type ActionCopy = {
  title: string
  description: string
  cta: string
  done: string
}

function actionCopy({ feature, target }: PendingFeatureAction): ActionCopy {
  if (target === FeatureStatus.DISABLED)
    return {
      title: 'Disable feature?',
      description:
        "Customers lose access while it's off. Settings stay saved — turning it back on restores everything.",
      cta: 'Disable',
      done: 'Feature disabled successfully',
    }
  if (target === FeatureStatus.ARCHIVED)
    return {
      title: 'Archive feature?',
      description:
        'Customers who have this feature lose access, and it is hidden with its entitlements until you unarchive it. Its code is freed for a new feature.',
      cta: 'Archive',
      done: 'Feature archived successfully',
    }
  if (feature.status === FeatureStatus.ARCHIVED)
    return {
      title: 'Unarchive feature?',
      description: 'The feature becomes active again, and its entitlements apply as they were.',
      cta: 'Unarchive',
      done: 'Feature unarchived successfully',
    }
  return {
    title: 'Enable feature?',
    description:
      'Customers regain access to this feature. All entitlement settings come back as they were.',
    cta: 'Enable',
    done: 'Feature enabled successfully',
  }
}

/** Confirms a feature status change. */
export const FeatureActionDialog = ({
  pending,
  onClose,
  onDone,
}: {
  pending: PendingFeatureAction | null
  onClose: () => void
  onDone?: () => void
}) => {
  const queryClient = useQueryClient()
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    setError(null)
  }, [pending])

  const copy = pending ? actionCopy(pending) : null
  // Unarchiving is refused while another feature uses the code, so say so before the attempt.
  const unarchiving = pending?.feature.status === FeatureStatus.ARCHIVED
  const holderQuery = useQuery(
    getFeatureCodeHolder,
    pending && unarchiving ? { id: pending.feature.id } : skipToken
  )
  const codeUsedBy = unarchiving ? holderQuery.data?.holder : undefined

  const mutation = useMutation(setFeatureStatus, {
    onSuccess: () => {
      for (const schema of [
        listFeatures,
        getFeature,
        getFeatureCodeHolder,
        listEntitlementsByEntity,
      ]) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: schema.parent, cardinality: undefined }),
        })
      }
      if (copy) toast.success(copy.done)
      onClose()
      onDone?.()
    },
    onError: err => setError(err.rawMessage),
  })

  return (
    <AlertDialog open={pending !== null} onOpenChange={open => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{copy?.title}</AlertDialogTitle>
          <AlertDialogDescription>{copy?.description}</AlertDialogDescription>
        </AlertDialogHeader>
        {codeUsedBy && (
          <p className="text-sm text-destructive">
            {codeUsedBy.name} already uses the code {codeUsedBy.code}.
          </p>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={
              mutation.isPending ||
              (unarchiving && holderQuery.isPending) ||
              codeUsedBy !== undefined
            }
            onClick={e => {
              e.preventDefault()
              if (!pending) return
              mutation.mutate({ id: pending.feature.id, status: pending.target })
            }}
          >
            {copy?.cta}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
