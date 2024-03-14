import { disableQuery, useMutation } from '@connectrpc/connect-query'
import { Button, Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom, useSetAtom } from 'jotai'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import ConfirmationModal from '@/components/ConfirmationModal'
import {
  addedComponentsAtom,
  editedComponentsAtom,
  useIsDraftVersion,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  copyVersionToDraft,
  discardDraftVersion,
  getLastPublishedPlanVersion,
  publishPlanVersion,
  listPlans,
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

  const { data: lastPublishedVersion } = useQuery(
    getLastPublishedPlanVersion,
    overview?.planId
      ? {
          planId: overview.planId,
        }
      : disableQuery,
    { enabled: isDraft }
  )

  const navigate = useNavigate()

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
    if (!overview) return
    setIsBusy(true)

    await discardDraftMutation.mutateAsync({
      planId: overview.planId,
      planVersionId: overview.planVersionId,
    })
    resetAtoms()

    if (!lastPublishedVersion?.version) {
      navigate('../')
    }

    // TODO if overview.lastPublishedPlanId, redirect to that one
  }

  const publishPlanMutation = useMutation(publishPlanVersion, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
    },
  })

  const publishPlan = async () => {
    setIsBusy(true)

    if (!overview) return

    await publishPlanMutation.mutateAsync({
      planId: overview.planId,
      planVersionId: overview.planVersionId,
    })
    setIsBusy(false)
  }

  const copyToDraftMutation = useMutation(copyVersionToDraft, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] })
    },
  })

  const copyToDraft = async () => {
    if (!overview) return
    await copyToDraftMutation.mutateAsync({
      planId: overview.planId,
      planVersionId: overview.planVersionId,
    })
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
      <Button variant="link" className=" py-1.5 !rounded-r-none" onClick={copyToDraft}>
        Draft New Version
      </Button>
    </>
  )
}
