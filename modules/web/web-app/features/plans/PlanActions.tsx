import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Modal,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom, useSetAtom } from 'jotai'
import { ArchiveIcon, ArchiveRestoreIcon, ChevronDown, PencilIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { addedComponentsAtom, editedComponentsAtom } from '@/features/plans/pricecomponents/utils'
import { useBasePath } from '@/hooks/useBasePath'
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

export const PlanActions = () => {
  const [addedComponents] = useAtom(addedComponentsAtom)
  const [editedComponents] = useAtom(editedComponentsAtom)
  const [isBusy, setIsBusy] = useState(false)
  const [isConfirmOpen, setConfirmOpen] = useState(false)
  const queryClient = useQueryClient()

  const wip = addedComponents.length > 0 || editedComponents.length > 0

  const overview = usePlanOverview()

  const isDraft = useIsDraftVersion()

  const planWithVersion = usePlanWithVersion()

  const navigate = useNavigate()
  const basePath = useBasePath()

  const setEditedComponents = useSetAtom(editedComponentsAtom)
  const setAddedComponents = useSetAtom(addedComponentsAtom)

  const resetAtoms = () => {
    setEditedComponents([])
    setAddedComponents([])
  }

  useEffect(() => {
    setIsBusy(false)
    return () => setIsBusy(false)
  }, [isDraft])

  const discardDraftMutation = useMutation(discardDraftVersion, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
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

    if (!overview?.activeVersion) {
      navigate('../')
    }

    // TODO if overview.lastPublishedPlanId, redirect to that one
  }

  const publishPlanMutation = useMutation(publishPlanVersion, {
    onSuccess: async res => {
      queryClient.invalidateQueries({ queryKey: [getPlanOverview.service.typeName] })
      queryClient.invalidateQueries({ queryKey: [getPlanWithVersion.service.typeName] })
      queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })

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
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
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
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getPlanWithVersion.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getPlanOverview.service.typeName] })
      toast.success('Plan archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive plan')
    },
  })

  const unarchiveMutation = useMutation(unarchivePlan, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getPlanWithVersion.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getPlanOverview.service.typeName] })
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

  return isDraft ? (
    <>
      <div className="text-muted-foreground text-xs  self-center">
        {wip ? 'Some components have not been saved' : ''}
      </div>
      <div className="flex ">
        <Button
          variant="destructiveGhost"
          className=" py-1.5 !rounded-r-none"
          onClick={confirmDiscardDraft}
          size="sm"
        >
          Discard draft
        </Button>
        <Button
          variant="primary"
          className="font-bold py-1.5 !rounded-l-none"
          disabled={wip || isBusy}
          onClick={publishPlan}
          size="sm"
        >
          Publish version
        </Button>
      </div>
      <ConfirmationModal
        visible={isConfirmOpen}
        danger
        header="Confirm to discard draft"
        buttonLabel="Confirm"
        onSelectCancel={() => setConfirmOpen(false)}
        onSelectConfirm={() => {
          discardDraft()
          setConfirmOpen(false)
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            Are you sure you want to discard this draft? Your changes will be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  ) : (
    <>
      <div className="flex gap-2">
        <Button variant="outline" hasIcon className="py-1.5" onClick={copyToDraft}>
          <PencilIcon size="12" /> New version
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" className="gap-2 py-1.5" size="sm" hasIcon>
              Actions <ChevronDown className="w-4 h-4" />
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
    </>
  )
}
