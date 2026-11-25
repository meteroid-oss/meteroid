import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import { Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { DetailsForm, createPlanSchema } from '@/features/plans/create/details/DetailsForm'
import { useTenant } from '@/hooks/useTenant'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

export const PlanCreateInitModal = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // A plan's prices name their currencies; the version keeps the account's as its fallback.
  const currency = useTenant().tenant?.reportingCurrency ?? ''

  const onCancel = () => navigate('..')

  const createPlanMutation = useMutation(createDraftPlan, {
    onSuccess: () => queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listPlans.parent,
      cardinality: undefined
    }) }),
  })

  const handleSubmit = async (values: z.infer<typeof createPlanSchema>) => {
    try {
      const plan = await createPlanMutation.mutateAsync({
        name: values.planName,
        description: values.description,
        // Free and sales-led are read from prices and where the plan is sold, not chosen here.
        planType: PlanType.STANDARD,
        productFamilyLocalId: values.productFamilyLocalId,
        currency,
      })

      navigate(`../${plan.plan?.plan?.localId}/draft`)
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Failed to create plan')
    }
  }

  return (
    <Modal
      layout="vertical"
      visible={true}
      header="Create a plan"
      size="large"
      onCancel={onCancel}
      hideFooter
    >
      <div className="px-5 py-4">
        <DetailsForm
          onCancel={onCancel}
          onNext={handleSubmit}
          submitLabel="Create plan"
          isSubmitting={createPlanMutation.isPending}
        />
      </div>
    </Modal>
  )
}
