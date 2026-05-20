import { useMutation } from '@connectrpc/connect-query'
import { Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { DetailsForm, createPlanSchema } from '@/features/plans/create/details/DetailsForm'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

export const PlanCreateInitModal = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const onCancel = () => navigate('..')

  const createPlanMutation = useMutation(createDraftPlan, {
    onSuccess: () => queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] }),
  })

  const handleSubmit = async (values: z.infer<typeof createPlanSchema>) => {
    try {
      const plan = await createPlanMutation.mutateAsync({
        name: values.planName,
        description: values.description,
        planType: PlanType[values.planType as keyof typeof PlanType],
        productFamilyLocalId: values.productFamilyLocalId,
        currency: values.currency,
      })

      if (values.planType === 'FREE') {
        navigate(`../${plan.plan?.plan?.localId}`)
      } else {
        navigate(`../${plan.plan?.plan?.localId}/draft`)
      }
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Failed to create plan')
    }
  }

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={<>Create a new plan </>}
      size="xlarge"
      onCancel={onCancel}
      hideFooter
    >
      <div className="px-5 py-4">
        <DetailsForm
          onCancel={onCancel}
          onNext={handleSubmit}
          submitLabel="Create Plan"
          isSubmitting={createPlanMutation.isPending}
        />
      </div>
    </Modal>
  )
}
