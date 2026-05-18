import { useMutation } from '@connectrpc/connect-query'
import { Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Wizard, useWizard } from 'react-use-wizard'
import { toast } from 'sonner'
import { z } from 'zod'

import { EntitlementCreationStep } from '@/features/entitlements/creation/EntitlementCreationStep'
import { resolveEntitlementSpecs } from '@/features/entitlements/creation/resolveEntitlementSpecs'
import { PendingEntitlementSpec } from '@/features/entitlements/creation/types'
import { DetailsForm, createPlanSchema } from '@/features/plans/create/details/DetailsForm'
import { createFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { createDraftPlan, listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

const Step1 = ({
  onCancel,
  onNext,
  step1Values,
}: {
  onCancel: () => void
  onNext: (v: z.infer<typeof createPlanSchema>) => void
  step1Values: z.infer<typeof createPlanSchema> | null
}) => {
  const { nextStep } = useWizard()
  return (
    <DetailsForm
      onCancel={onCancel}
      defaultValues={step1Values ?? undefined}
      onNext={values => {
        onNext(values)
        nextStep()
      }}
    />
  )
}

const Step2 = ({
  pendingEntitlements,
  onSubmit,
  isSubmitting,
}: {
  pendingEntitlements: PendingEntitlementSpec[]
  onSubmit: (entitlements: PendingEntitlementSpec[]) => Promise<void>
  isSubmitting: boolean
}) => {
  const { previousStep } = useWizard()
  return (
    <EntitlementCreationStep
      initialEntitlements={pendingEntitlements}
      submitLabel="Create Plan"
      onBack={previousStep}
      onSubmit={onSubmit}
      isSubmitting={isSubmitting}
    />
  )
}

export const PlanCreateInitModal = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [step1Values, setStep1Values] = useState<z.infer<typeof createPlanSchema> | null>(null)
  const [pendingEntitlements, setPendingEntitlements] = useState<PendingEntitlementSpec[]>([])

  const onCancel = () => navigate('..')

  const createPlanMutation = useMutation(createDraftPlan, {
    onSuccess: () => queryClient.invalidateQueries({ queryKey: [listPlans.service.typeName] }),
  })
  const createFeatureMutation = useMutation(createFeature)

  const handleEntitlementsSubmit = async (entitlements: PendingEntitlementSpec[]) => {
    if (!step1Values) return
    setPendingEntitlements(entitlements)

    try {
      const resolved = await resolveEntitlementSpecs(entitlements, req =>
        createFeatureMutation.mutateAsync(req)
      )

      const plan = await createPlanMutation.mutateAsync({
        name: step1Values.planName,
        description: step1Values.description,
        planType: PlanType[step1Values.planType as keyof typeof PlanType],
        productFamilyLocalId: step1Values.productFamilyLocalId,
        currency: step1Values.currency,
        entitlements: resolved,
      })

      if (step1Values.planType === 'FREE') {
        navigate(`../${plan.plan?.plan?.localId}`)
      } else {
        navigate(`../${plan.plan?.plan?.localId}/draft`)
      }
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Failed to create plan')
    }
  }

  const isSubmitting = createPlanMutation.isPending || createFeatureMutation.isPending

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
        <Wizard>
          <Step1 onCancel={onCancel} onNext={setStep1Values} step1Values={step1Values} />
          <Step2
            pendingEntitlements={pendingEntitlements}
            onSubmit={handleEntitlementsSubmit}
            isSubmitting={isSubmitting}
          />
        </Wizard>
      </div>
    </Modal>
  )
}
