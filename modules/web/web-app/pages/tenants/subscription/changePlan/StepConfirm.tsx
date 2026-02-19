import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, CardContent, CardHeader, CardTitle } from '@md/ui'
import { useAtom } from 'jotai'
import { ArrowRight, Calendar } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  PricingComponent,
  SubscriptionPricingTable,
} from '@/features/subscriptions/pricecomponents/SubscriptionPricingTable'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { schedulePlanChange } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

export const StepConfirm = () => {
  const navigate = useNavigate()
  const { previousStep } = useWizard()
  const [state] = useAtom(changePlanAtom)

  const scheduleMut = useMutation(schedulePlanChange)

  const preview = state.preview
  const currency = state.currency || 'USD'

  const newComponents: PricingComponent[] = useMemo(() => {
    if (!preview) return []
    const matched = preview.matched.map((m, idx) => ({
      id: m.productId || `matched-${idx}`,
      name: m.newName,
      period: m.newPeriod,
      fee: m.newFee,
    }))
    const added = preview.added.map((a, idx) => ({
      id: `added-${idx}`,
      name: a.name,
      period: a.period,
      fee: a.fee,
    }))
    return [...matched, ...added]
  }, [preview])

  const handleSchedule = async () => {
    if (!state.targetPlanVersionId || !state.subscriptionId) return

    try {
      const result = await scheduleMut.mutateAsync({
        subscriptionId: state.subscriptionId,
        newPlanVersionId: state.targetPlanVersionId,
      })
      toast.success(
        `Plan change scheduled for ${result.effectiveDate ? parseAndFormatDate(result.effectiveDate) : 'end of current period'}`
      )
      navigate('..')
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to schedule plan change'
      toast.error(message)
    }
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Confirm Plan Change',
          subtitle: 'Review and schedule the plan change',
        }}
      >
        <div className="space-y-6">
          <Card>
            <CardHeader className="flex flex-row items-center gap-2">
              <ArrowRight className="h-5 w-5" />
              <CardTitle className="text-base">Plan Change</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="flex items-center gap-4">
                <div className="flex-1 rounded-lg border border-border p-3">
                  <div className="text-xs text-muted-foreground">Current Plan</div>
                  <div className="text-sm font-medium text-foreground">{state.currentPlanName}</div>
                </div>
                <ArrowRight className="h-5 w-5 text-muted-foreground flex-shrink-0" />
                <div className="flex-1 rounded-lg border border-brand/30 bg-brand/5 p-3">
                  <div className="text-xs text-muted-foreground">New Plan</div>
                  <div className="text-sm font-medium text-foreground">
                    {state.targetPlanName}
                  </div>
                </div>
              </div>
            </CardContent>
          </Card>

          {preview?.effectiveDate && (
            <Card>
              <CardHeader className="flex flex-row items-center gap-2">
                <Calendar className="h-5 w-5" />
                <CardTitle className="text-base">Effective Date</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-sm font-medium text-foreground">
                  {parseAndFormatDate(preview.effectiveDate)}
                </div>
                <div className="text-xs text-muted-foreground mt-1">
                  The change will take effect at the end of the current billing period.
                </div>
              </CardContent>
            </Card>
          )}

          <SubscriptionPricingTable
            components={newComponents}
            currency={currency}
            labelClassName="px-4 py-3"
          />
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep} disabled={scheduleMut.isPending}>
          Back
        </Button>
        <Button
          variant="brand"
          onClick={handleSchedule}
          disabled={scheduleMut.isPending}
          className="min-w-[180px]"
        >
          {scheduleMut.isPending ? 'Scheduling...' : 'Schedule Plan Change'}
        </Button>
      </div>
    </div>
  )
}
