import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Label,
  RadioGroup,
  RadioGroupItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom } from 'jotai'
import { AlertTriangle, ArrowRight, Calendar, Zap } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  PricingComponent,
  SubscriptionPricingTable,
} from '@/features/subscriptions/pricecomponents/SubscriptionPricingTable'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrency } from '@/lib/utils/numbers'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import {
  ScheduledEventType,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { PlanChangeApplyMode } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import {
  getSubscriptionDetails,
  schedulePlanChange,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

const scheduledEventLabel = (eventType: ScheduledEventType): string => {
  switch (eventType) {
    case ScheduledEventType.PLAN_CHANGE:
      return 'Pending plan change'
    case ScheduledEventType.CANCEL:
      return 'Pending cancellation'
    case ScheduledEventType.PAUSE:
      return 'Pending pause'
    case ScheduledEventType.END_TRIAL:
      return 'Pending trial end'
    default:
      return 'Pending event'
  }
}

export const StepConfirm = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { previousStep } = useWizard()
  const [state, setState] = useAtom(changePlanAtom)
  const [forceAnnual, setForceAnnual] = useState(false)

  const scheduleMut = useMutation(schedulePlanChange)

  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )
  const pendingEvents = (subscriptionQuery.data?.pendingEvents ?? []).filter(
    e => e.eventType !== ScheduledEventType.END_TRIAL
  )

  const preview = state.preview
  const currency = state.currency || 'USD'
  const isDowngrade = preview?.changeDirection === 'downgrade'
  const isImmediate = state.applyMode === PlanChangeApplyMode.IMMEDIATE

  // Detect if subscription has annual components
  const isAnnual = (subscriptionQuery.data?.priceComponents ?? []).some(
    c => c.period === SubscriptionFeeBillingPeriod.YEARLY
  )

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

  const handleModeChange = (value: string) => {
    const mode =
      value === 'immediate'
        ? PlanChangeApplyMode.IMMEDIATE
        : PlanChangeApplyMode.END_OF_PERIOD
    setState(prev => ({ ...prev, applyMode: mode }))
  }

  const handleSchedule = async () => {
    if (!state.targetPlanVersionId || !state.subscriptionId) return

    try {
      const result = await scheduleMut.mutateAsync({
        subscriptionId: state.subscriptionId,
        newPlanVersionId: state.targetPlanVersionId,
        applyMode: state.applyMode,
        forceAnnual,
      })

      if (isImmediate) {
        toast.success(
          `Plan change applied immediately${result.invoiceId ? ' — adjustment invoice created' : ''}`
        )
      } else {
        toast.success(
          `Plan change scheduled for ${result.effectiveDate ? parseAndFormatDate(result.effectiveDate) : 'end of current period'}`
        )
      }

      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getSubscriptionDetails, {
          subscriptionId: state.subscriptionId,
        }),
      })
      navigate('..')
    } catch (error) {
      const message =
        error instanceof Error ? error.message : 'Failed to schedule plan change'
      toast.error(message)
    }
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Confirm Plan Change',
          subtitle: 'Review and apply the plan change',
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

          {/* Apply mode selector */}
          <Card>
            <CardHeader className="flex flex-row items-center gap-2">
              <Zap className="h-5 w-5" />
              <CardTitle className="text-base">When to apply</CardTitle>
            </CardHeader>
            <CardContent>
              <RadioGroup
                value={isImmediate ? 'immediate' : 'end_of_period'}
                onValueChange={handleModeChange}
                className="space-y-3"
              >
                <div className="flex items-start gap-3">
                  <RadioGroupItem value="end_of_period" id="mode-eop" className="mt-0.5" />
                  <Label htmlFor="mode-eop" className="cursor-pointer">
                    <div className="text-sm font-medium">At end of billing period</div>
                    <div className="text-xs text-muted-foreground">
                      The change will take effect at the next renewal date.
                    </div>
                  </Label>
                </div>
                <div
                  className="flex items-start gap-3"
                  title={isDowngrade ? 'Downgrades can only be applied at end of period' : undefined}
                >
                  <RadioGroupItem
                    value="immediate"
                    id="mode-immediate"
                    className="mt-0.5"
                    disabled={isDowngrade}
                  />
                  <Label
                    htmlFor="mode-immediate"
                    className={`cursor-pointer ${isDowngrade ? 'opacity-50' : ''}`}
                  >
                    <div className="text-sm font-medium">Immediately</div>
                    <div className="text-xs text-muted-foreground">
                      Apply now with a prorated adjustment invoice.
                      {isDowngrade && (
                        <span className="block text-xs text-destructive mt-0.5">
                          Downgrades can only be applied at end of period.
                        </span>
                      )}
                    </div>
                  </Label>
                </div>
              </RadioGroup>

              {isImmediate && isAnnual && (
                <div className="mt-4">
                  <Alert variant="warning">
                    <div className="flex items-start gap-2">
                      <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                      <div>
                        <span className="text-sm font-medium">
                          This is an annual subscription.
                        </span>
                        <p className="text-xs mt-1">
                          Immediate changes to annual plans result in a large prorated adjustment.
                        </p>
                        <label className="flex items-center gap-2 mt-2 cursor-pointer">
                          <input
                            type="checkbox"
                            checked={forceAnnual}
                            onChange={e => setForceAnnual(e.target.checked)}
                            className="rounded border-border"
                          />
                          <span className="text-xs">I understand, proceed anyway</span>
                        </label>
                      </div>
                    </div>
                  </Alert>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Proration preview */}
          {isImmediate && preview?.proration && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Proration Summary</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Credits (unused old plan)</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.creditsTotalCents, currency)}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Charges (new plan remainder)</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.chargesTotalCents, currency)}
                    </span>
                  </div>
                  <div className="border-t border-border my-2" />
                  <div className="flex justify-between font-medium">
                    <span className="text-foreground">Net adjustment</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.netAmountCents, currency)}
                    </span>
                  </div>
                  <div className="text-xs text-muted-foreground mt-2">
                    {preview.proration.daysRemaining} of {preview.proration.daysInPeriod} days
                    remaining in current period (
                    {Math.round(preview.proration.prorationFactor * 100)}% prorated)
                  </div>
                </div>
              </CardContent>
            </Card>
          )}

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
                  {isImmediate
                    ? 'The change will be applied immediately.'
                    : 'The change will take effect at the end of the current billing period.'}
                </div>
              </CardContent>
            </Card>
          )}

          <SubscriptionPricingTable
            components={newComponents}
            currency={currency}
            labelClassName="px-4 py-3"
          />

          {pendingEvents.length > 0 && (
            <Alert variant="warning">
              <div className="flex items-start gap-2">
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <div>
                  <span className="text-sm font-medium">
                    {isImmediate
                      ? 'Applying this plan change will cancel the following pending events:'
                      : 'Scheduling this plan change will cancel the following pending events:'}
                  </span>
                  <ul className="text-sm mt-1 list-disc list-inside">
                    {pendingEvents.map(event => (
                      <li key={event.id}>
                        {scheduledEventLabel(event.eventType)}
                        {event.newPlanName ? ` to "${event.newPlanName}"` : ''}
                        {event.scheduledDate
                          ? ` on ${parseAndFormatDate(event.scheduledDate)}`
                          : ''}
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            </Alert>
          )}
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep} disabled={scheduleMut.isPending}>
          Back
        </Button>
        <Button
          variant="brand"
          onClick={handleSchedule}
          disabled={scheduleMut.isPending || (isImmediate && isAnnual && !forceAnnual)}
          className="min-w-[180px]"
        >
          {scheduleMut.isPending
            ? isImmediate
              ? 'Applying...'
              : 'Scheduling...'
            : isImmediate
              ? 'Apply Plan Change Now'
              : 'Schedule Plan Change'}
        </Button>
      </div>
    </div>
  )
}
