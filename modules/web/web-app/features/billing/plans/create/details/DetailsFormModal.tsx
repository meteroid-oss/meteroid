import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField, Modal, Spinner, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FC } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { usePlanOverview } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { editPlanSchema } from '@/lib/schemas/plans'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanByLocalId,
  updateDraftPlanOverview,
  updatePublishedPlanOverview,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

export const DetailsFormModal: FC = () => {
  const navigate = useNavigate()
  const plan = usePlanOverview()

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={
        <>
          <>Update details</>
        </>
      }
      hideFooter
      size="xlarge"
      onCancel={() => navigate('..')}
    >
      {!!plan && <BasicDetailedForm plan={plan} />}
    </Modal>
  )
}

interface Props {
  plan: PlanOverview
}

const BasicDetailedForm = ({ plan }: Props) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const methods = useZodForm({
    schema: editPlanSchema,
    defaultValues: {
      description: plan.description,
      planName: plan.name,
      netTerms: plan.netTerms,
    },
  })

  const updatePublishedPlan = useMutation(updatePublishedPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [getPlanByLocalId.service.typeName],
      })
    },
  })

  const updateDraftPlan = useMutation(updateDraftPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [getPlanByLocalId.service.typeName],
      })
    },
  })

  const isLoading = updatePublishedPlan.isPending || updateDraftPlan.isPending

  const onSubmit = async (data: z.infer<typeof editPlanSchema>) => {
    if (plan.isDraft) {
      updateDraftPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        netTerms: data.netTerms,
        planId: plan.planId,
        planVersionId: plan.planVersionId,
        currency: plan.currency,
      })
    } else {
      await updatePublishedPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        planId: plan.planId,
        planVersionId: plan.planVersionId,
      })
    }

    navigate('..')
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <div className="px-6 pb-2">
          <div className="py-4 text-sm text-foreground flex flex-col w-full">
            <span className="flex w-full justify-between">
              <span>Plan details</span>
              <span className="bg-muted text-muted-foreground p-1 rounded-sm ml-4 text-xs">
                affects all versions of this plan
              </span>
            </span>
          </div>
          <div className="pl-2">
            <div className="space-y-6 py-2">
              <InputFormField
                name="planName"
                label="Name"
                layout="horizontal"
                control={methods.control}
                type="text"
                placeholder="Plan name"
              />
              <TextareaFormField
                name="description"
                label="Description"
                control={methods.control}
                placeholder="This plan gives access to ..."
                layout="horizontal"
              />
            </div>
          </div>
        </div>
        {plan.isDraft && (
          <>
            <div className="px-6 border-t pb-2">
              <div className="py-4 text-sm text-foreground flex flex-col">
                <span>Version details</span>
              </div>
              <div className="pl-2">
                <div className="space-y-6 py-2">
                  <InputFormField
                    name="netTerms"
                    control={methods.control}
                    label="Net terms (days)"
                    type="number"
                    placeholder="30"
                    layout="horizontal"
                    min={0}
                    max={180}
                    step={1}
                    className="w-[100px]"
                  />
                </div>
              </div>
            </div>
          </>
        )}
        <div className="flex justify-end gap-2 py-3 px-5 border-t ">
          <div className="flex w-full space-x-2 justify-end">
            <Button
              variant="secondary"
              onClick={() => navigate('..')}
              disabled={isLoading}
              size="sm"
              type="button"
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isLoading} hasIcon={isLoading} size="sm">
              {isLoading && <Spinner />}
              <>Save</>
            </Button>
          </div>
        </div>
      </form>
    </Form>
  )
}
