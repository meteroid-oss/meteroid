import { useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Skeleton } from '@md/ui'
import { useAtom } from 'jotai'
import { useEffect } from 'react'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { TargetChargesTable } from '@/features/subscriptions/planChange/TargetChargesTable'
import { ComponentChoiceControls } from '@/pages/tenants/subscription/changePlan/ComponentChoiceControls'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { usePlanChangeParameters } from '@/pages/tenants/subscription/changePlan/usePlanChangeParameters'
import { previewPlanChange } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'

const DirectionBadge = ({ direction }: { direction: string }) => {
  switch (direction) {
    case 'upgrade':
      return (
        <Badge variant="default" size="sm">
          Upgrade
        </Badge>
      )
    case 'downgrade':
      return (
        <Badge variant="secondary" size="sm">
          Downgrade
        </Badge>
      )
    default:
      return (
        <Badge variant="outline" size="sm">
          Lateral
        </Badge>
      )
  }
}

export const StepReviewMapping = () => {
  const { previousStep, nextStep } = useWizard()
  const [state, setState] = useAtom(changePlanAtom)

  const previewMut = useMutation(previewPlanChange)
  const {
    configurable,
    matchedConfigurable,
    parameterizedComponents,
    prices,
    parametersKey,
    choose,
    setPrice,
    ready,
  } = usePlanChangeParameters()

  useEffect(() => {
    if (ready && state.targetPlanVersionId && state.subscriptionId) {
      previewMut.mutate(
        {
          subscriptionId: state.subscriptionId,
          newPlanVersionId: state.targetPlanVersionId,
          applyMode: state.applyMode,
          parameterizedComponents,
          option: state.option,
          prices,
        },
        {
          onSuccess: data => {
            setState(prev => ({
              ...prev,
              preview: data,
            }))
          },
        }
      )
    }
  }, [ready, state.targetPlanVersionId, state.subscriptionId, state.applyMode, parametersKey])

  const preview = state.preview
  const currency = state.currency || 'USD'

  // A target component's choices: its seats or capacity when added, its level when matched.
  const controlsFor = (componentId: string | undefined, fee?: SubscriptionFee) => {
    const choices =
      configurable.find(c => c.component.id === componentId) ??
      matchedConfigurable.find(c => c.component.id === componentId)
    if (!choices) return null
    return (
      <ComponentChoiceControls
        configurable={choices}
        currency={currency}
        slotUnit={fee?.fee.case === 'slot' ? fee.fee.value.unit : undefined}
        onChange={choice => choose(choices.component.id, choice)}
      />
    )
  }

  if (!preview && previewMut.isError) {
    return (
      <div className="space-y-6">
        <PageSection
          header={{
            title: 'Review Component Mapping',
            subtitle: 'Failed to load preview',
          }}
        >
          <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4">
            <div className="text-sm text-destructive">
              {previewMut.error?.message ?? 'An error occurred while loading the preview.'}
            </div>
          </div>
        </PageSection>
        <div className="flex gap-2 justify-end">
          <Button variant="secondary" onClick={previousStep}>
            Back
          </Button>
        </div>
      </div>
    )
  }

  if (!preview) {
    return (
      <div className="space-y-6">
        <PageSection
          header={{
            title: 'Review Component Mapping',
            subtitle: 'Loading preview...',
          }}
        >
          <div className="space-y-4">
            <Skeleton height={60} />
            <Skeleton height={60} />
            <Skeleton height={60} />
          </div>
        </PageSection>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Review Component Mapping',
          subtitle: `${state.currentPlanName} → ${state.targetPlanName}`,
        }}
      >
        {previewMut.isError && (
          <div className="mb-4 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
            {previewMut.error?.message ?? 'An error occurred while loading the preview.'}
          </div>
        )}

        {preview.changeDirection && (
          <div className="mb-6">
            <DirectionBadge direction={preview.changeDirection} />
          </div>
        )}

        <TargetChargesTable
          preview={preview}
          currency={currency}
          pending={previewMut.isPending}
          prices={state.prices}
          onPrice={setPrice}
          controls={controlsFor}
        />
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep}>
          Back
        </Button>
        <Button
          variant="primary"
          onClick={nextStep}
          disabled={previewMut.isPending || previewMut.isError}
        >
          Next
        </Button>
      </div>
    </div>
  )
}
