import { Button, Label } from '@md/ui'
import { cn } from '@ui/lib'
import { useAtom } from 'jotai'
import { useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { OptionSelect } from '@/features/subscriptions/pricecomponents/OptionSelect'
import {
  byPosition,
  findOption,
  optionHandle,
} from '@/features/subscriptions/pricecomponents/planOption'
import { useQuery } from '@/lib/connectrpc'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { PlanChangeApplyMode } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

import type { PlanOverview } from '@/rpc/api/plans/v1/models_pb'

/** The subscription's own option where the target sells it in its currency, else the first sold. */
const defaultOptionOf = (plan: PlanOverview, currency: string, current?: string) => {
  const options = byPosition(plan.options)
  const sold = options.filter(o =>
    plan.pricedOptions.some(p => p.optionId === o.id && p.currency === currency)
  )
  const option = findOption(sold, current) ?? sold[0] ?? options[0]
  return option && optionHandle(option)
}

export const StepSelectPlan = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(changePlanAtom)
  const [selectedVersionId, setSelectedVersionId] = useState<string | undefined>(
    state.targetPlanVersionId
  )

  const plansQuery = useQuery(listPlans, {
    filters: {
      types: [PlanType.STANDARD, PlanType.FREE],
      statuses: [PlanStatus.ACTIVE],
      currency: state.currency || undefined,
    },
  })

  const plans = plansQuery.data?.plans ?? []

  const eligiblePlans = plans.filter(plan => {
    if (!plan.activeVersion) return false
    if (plan.activeVersion.id === state.currentPlanVersionId) return false
    return true
  })

  const handleSelect = (plan: PlanOverview) => {
    const versionId = plan.activeVersion!.id
    setSelectedVersionId(versionId)
    setState(prev => ({
      ...prev,
      targetPlanVersionId: versionId,
      targetPlanName: plan.name,
      option: defaultOptionOf(plan, prev.currency, prev.currentOption),
      preview: undefined,
      componentChoices: {},
      // A new target may forbid an immediate change: its preview starts at the period's end.
      applyMode: PlanChangeApplyMode.END_OF_PERIOD,
    }))
  }

  // `?targetPlanVersionId=` (e.g. from a signal's option) preselects the target and moves on.
  const [params] = useSearchParams()
  const preset = params.get('targetPlanVersionId')
  const presetApplied = useRef(false)
  useEffect(() => {
    if (!preset || presetApplied.current || !state.currentPlanVersionId) return
    const target = eligiblePlans.find(plan => plan.activeVersion?.id === preset)
    if (!target) return
    presetApplied.current = true
    handleSelect(target)
    nextStep()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preset, state.currentPlanVersionId, eligiblePlans.length])

  const selectedPlan = eligiblePlans.find(plan => plan.activeVersion?.id === selectedVersionId)
  // The added components' prices and cadence are this option's: their choices start over.
  const chooseOption = (handle: string) =>
    setState(prev => ({ ...prev, option: handle, preview: undefined, componentChoices: {} }))

  const handleNext = () => {
    if (selectedVersionId) {
      nextStep()
    }
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Select Target Plan',
          subtitle: `Current plan: ${state.currentPlanName}`,
        }}
      >
        {plansQuery.isLoading ? (
          <div className="text-sm text-muted-foreground">Loading plans...</div>
        ) : eligiblePlans.length === 0 ? (
          <div className="text-sm text-muted-foreground">
            No eligible plans available for plan change.
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {eligiblePlans.map(plan => (
              <div
                key={plan.activeVersion!.id}
                onClick={() => handleSelect(plan)}
                className={cn(
                  'cursor-pointer rounded-lg border p-4 transition-colors',
                  selectedVersionId === plan.activeVersion!.id
                    ? 'border-brand bg-brand/5'
                    : 'border-border hover:border-brand/50'
                )}
              >
                <div className="font-medium text-foreground">{plan.name}</div>
                <div className="text-sm text-muted-foreground">
                  Version {plan.activeVersion!.version}
                </div>
                {plan.description && (
                  <div className="text-sm text-muted-foreground mt-1">{plan.description}</div>
                )}
              </div>
            ))}
          </div>
        )}
        {selectedPlan && selectedPlan.options.length > 1 && (
          <div className="mt-4 flex max-w-md items-center gap-3">
            <Label className="text-sm text-muted-foreground">Option</Label>
            <OptionSelect
              options={selectedPlan.options}
              value={findOption(selectedPlan.options, state.option)}
              onChange={chooseOption}
              className="w-64"
            />
          </div>
        )}
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="primary" onClick={handleNext} disabled={!selectedVersionId}>
          Next
        </Button>
      </div>
    </div>
  )
}
