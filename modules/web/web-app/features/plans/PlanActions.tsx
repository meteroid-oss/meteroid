import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Modal,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom } from 'jotai'
import { ArchiveIcon, ArchiveRestoreIcon, HistoryIcon, MoreHorizontalIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { editedComponentsAtom } from '@/features/plans/pricecomponents/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { PlanStatus } from '@/rpc/api/plans/v1/models_pb'
import {
  archivePlan,
  copyVersionToDraft,
  discardDraftVersion,
  getPlanOverview,
  getPlanWithVersion,
  listPlans,
  publishPlanVersion,
  unarchivePlan,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

/** The header's actions: a draft is discarded or published, a published version drafted from. */
export const PlanActions = ({ onHistory }: { onHistory?: () => void }) => {
  const isExpress = useIsExpressOrganization()
  const canManage = usePermissions().has('catalog:manage')
  const [editedComponents] = useAtom(editedComponentsAtom)
  const [isBusy, setIsBusy] = useState(false)
  const [isConfirmOpen, setConfirmOpen] = useState(false)
  const [isPublishOpen, setPublishOpen] = useState(false)
  const queryClient = useQueryClient()

  const wip = editedComponents.length > 0

  const overview = usePlanOverview()

  const isDraft = useIsDraftVersion()

  const planWithVersion = usePlanWithVersion()

  const navigate = useNavigate()
  const basePath = useBasePath()

  const [, setEditedComponents] = useAtom(editedComponentsAtom)

  const resetAtoms = () => {
    setEditedComponents([])
  }

  useEffect(() => {
    setIsBusy(false)
    return () => setIsBusy(false)
  }, [isDraft])

  const discardDraftMutation = useMutation(discardDraftVersion, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
    },
  })

  const confirmDiscardDraft = () => {
    setConfirmOpen(true)
  }

  const discardDraft = async () => {
    if (!overview || !planWithVersion.plan || !planWithVersion.version) return
    setIsBusy(true)

    await discardDraftMutation.mutateAsync({
      planId: planWithVersion.plan.id,
      planVersionId: planWithVersion.version.id,
    })
    resetAtoms()

    if (overview?.activeVersion) {
      navigate(`${basePath}/plans/${planWithVersion.plan.id}`)
    } else {
      navigate(`${basePath}/plans`)
    }
  }

  const publishPlanMutation = useMutation(publishPlanVersion, {
    onSuccess: async res => {
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanOverview.parent,
        cardinality: undefined
      }) })
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanWithVersion.parent,
        cardinality: undefined
      }) })
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listAddOns.parent,
        cardinality: undefined
      }) })

      const version = res.planVersion?.version
      setIsBusy(false)

      navigate(`${basePath}/plans/${planWithVersion.plan?.id}/${version}`)
    },
  })

  const publishPlan = async () => {
    setIsBusy(true)

    if (!overview || !planWithVersion.plan || !planWithVersion.version) return

    await publishPlanMutation.mutateAsync({
      planId: planWithVersion.plan.id,
      planVersionId: planWithVersion.version.id,
    })
  }

  const copyToDraftMutation = useMutation(copyVersionToDraft, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
    },
  })

  const copyToDraft = async () => {
    if (!overview || !planWithVersion.plan || !planWithVersion.version) return
    await copyToDraftMutation.mutateAsync({
      planId: planWithVersion.plan.id,
      planVersionId: planWithVersion.version.id,
    })

    navigate(`${basePath}/plans/${planWithVersion.plan.id}/draft`)
  }

  const archiveMutation = useMutation(archivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanWithVersion.parent,
        cardinality: undefined
      }) })
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanOverview.parent,
        cardinality: undefined
      }) })
      toast.success('Plan archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive plan')
    },
  })

  const unarchiveMutation = useMutation(unarchivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listPlans.parent,
        cardinality: undefined
      }) })
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanWithVersion.parent,
        cardinality: undefined
      }) })
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getPlanOverview.parent,
        cardinality: undefined
      }) })
      toast.success('Plan unarchived successfully')
    },
    onError: () => {
      toast.error('Failed to unarchive plan')
    },
  })

  const handleArchive = () => {
    if (planWithVersion.plan) {
      archiveMutation.mutate({ id: planWithVersion.plan.id })
    }
  }

  const handleUnarchive = () => {
    if (planWithVersion.plan) {
      unarchiveMutation.mutate({ id: planWithVersion.plan.id })
    }
  }

  const isOutdatedDraft = isDraft && planWithVersion.version?.usesProductPricing === false

  if (isExpress || !canManage) return null

  const number = planWithVersion.version?.version
  const live = overview?.activeVersion
  const subscriptions = overview?.subscriptionCount ?? 0

  return isDraft ? (
    <>
      {wip && (
        <div className="text-warning text-xs self-center">A charge being edited is not saved</div>
      )}
      <div className="flex gap-2">
        <Button variant="outline" size="sm" onClick={confirmDiscardDraft}>
          Discard draft
        </Button>
        {!isOutdatedDraft && (
          <Button
            variant="primary"
            size="sm"
            disabled={wip || isBusy}
            onClick={() => setPublishOpen(true)}
          >
            Review &amp; publish{number ? ` v${number}` : ''}
          </Button>
        )}
      </div>
      <ConfirmationModal
        visible={isPublishOpen}
        header={`Publish${number ? ` v${number}` : ' this version'}`}
        buttonLabel="Publish"
        onSelectCancel={() => setPublishOpen(false)}
        onSelectConfirm={() => {
          setPublishOpen(false)
          void publishPlan().catch(() => setIsBusy(false))
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            New subscriptions get{number ? ` v${number}` : ' this version'} and its prices.
            {live && subscriptions > 0
              ? ` The ${subscriptions} ${subscriptions === 1 ? 'subscription' : 'subscriptions'} on v${live.version} stay on it until you migrate them.`
              : ''}
          </p>
        </Modal.Content>
      </ConfirmationModal>
      <ConfirmationModal
        visible={isConfirmOpen}
        danger
        header="Discard this draft"
        buttonLabel="Discard"
        onSelectCancel={() => setConfirmOpen(false)}
        onSelectConfirm={() => {
          discardDraft()
          setConfirmOpen(false)
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            Its changes are lost. The published version stays as it is.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  ) : (
    <div className="flex gap-2">
      {onHistory && (
        <Button variant="outline" size="sm" hasIcon onClick={onHistory}>
          <HistoryIcon size={14} /> Version history
        </Button>
      )}
      {overview?.hasDraftVersion ? (
        <Button
          variant="primary"
          size="sm"
          onClick={() => navigate(`${basePath}/plans/${overview.localId}/draft`)}
        >
          Continue the draft
        </Button>
      ) : (
        <Button variant="primary" size="sm" onClick={copyToDraft}>
          Edit prices in a draft
        </Button>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" className="size-8" aria-label="Plan actions">
            <MoreHorizontalIcon size={16} />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {planWithVersion.plan?.planStatus === PlanStatus.ARCHIVED ? (
            <DropdownMenuItem onClick={handleUnarchive}>
              <ArchiveRestoreIcon size={16} className="mr-2" />
              Unarchive
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onClick={handleArchive}>
              <ArchiveIcon size={16} className="mr-2" />
              Archive
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
