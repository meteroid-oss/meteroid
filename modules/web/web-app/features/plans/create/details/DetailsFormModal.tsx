import { useMutation } from '@connectrpc/connect-query'
import { Button, Form, InputFormField, Modal, Spinner, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { FC } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useZodForm } from '@/hooks/useZodForm'
import { editPlanSchema } from '@/lib/schemas/plans'
import { Plan, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  updateDraftPlanOverview,
  updatePublishedPlanOverview,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

export const DetailsFormModal: FC = () => {
  const navigate = useNavigate()
  const { version, plan } = usePlanWithVersion()

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
      {version && plan && <BasicDetailedForm version={version} plan={plan} />}
    </Modal>
  )
}

interface Props {
  plan: Plan
  version: PlanVersion
}

const BasicDetailedForm = ({ plan, version }: Props) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const isDraft = useIsDraftVersion()

  const methods = useZodForm({
    schema: editPlanSchema,
    defaultValues: {
      description: plan?.description,
      planName: plan.name,
      netTerms: version.netTerms,
      selfServiceRank: plan?.selfServiceRank ?? undefined,
    },
  })

  const updatePublishedPlan = useMutation(updatePublishedPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [getPlanOverview.service.typeName],
      })
    },
  })

  const updateDraftPlan = useMutation(updateDraftPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [getPlanOverview.service.typeName],
      })
    },
  })

  const isLoading = updatePublishedPlan.isPending || updateDraftPlan.isPending

  const onSubmit = async (data: z.infer<typeof editPlanSchema>) => {
    if (isDraft) {
      updateDraftPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        netTerms: data.netTerms,
        planId: plan.id,
        planVersionId: version.id,
        currency: version.currency,
      })
    } else {
      await updatePublishedPlan.mutateAsync({
        name: data.planName,
        description: data.description,
        planId: plan.id,
        planVersionId: version.id,
        selfServiceRank: data.selfServiceRank || 0,
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
              {!isDraft && (
                <InputFormField
                  name="selfServiceRank"
                  label="Self-service rank"
                  layout="horizontal"
                  control={methods.control}
                  type="number"
                  placeholder="Not set"
                  min={0}
                  step={1}
                  className="w-[100px]"
                />
              )}
            </div>
          </div>
        </div>
        {isDraft && (
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
