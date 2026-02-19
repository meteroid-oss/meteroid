import { Button } from '@md/ui'
import { cn } from '@ui/lib'
import { useAtom } from 'jotai'
import { useState } from 'react'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useQuery } from '@/lib/connectrpc'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

export const StepSelectPlan = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(changePlanAtom)
  const [selectedVersionId, setSelectedVersionId] = useState<string | undefined>(
    state.targetPlanVersionId
  )

  const plansQuery = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
    filters: {
      types: [PlanType.STANDARD],
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

  const handleSelect = (versionId: string, planName: string) => {
    setSelectedVersionId(versionId)
    setState(prev => ({
      ...prev,
      targetPlanVersionId: versionId,
      targetPlanName: planName,
      preview: undefined,
    }))
  }

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
                onClick={() => handleSelect(plan.activeVersion!.id, plan.name)}
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
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="primary" onClick={handleNext} disabled={!selectedVersionId}>
          Next
        </Button>
      </div>
    </div>
  )
}
