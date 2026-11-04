import { useMutation } from '@connectrpc/connect-query'
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
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { invalidateSubscriptionBilling } from '@/features/subscriptions/invalidateSubscriptionBilling'
import { formatPackLineCredit, packLineView } from '@/features/subscriptions/planChangePackLines'
import {
  PricingComponent,
  SubscriptionPricingTable,
} from '@/features/subscriptions/pricecomponents/SubscriptionPricingTable'
import { creditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatCurrency } from '@/lib/utils/numbers'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { usePlanChangeParameters } from '@/pages/tenants/subscription/changePlan/usePlanChangeParameters'
import {
  ScheduledEventType,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getSubscriptionDetails,
  previewPlanChange,
  schedulePlanChange,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { PlanChangeApplyMode } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import { parseAndFormatDate } from '@/utils/date'

import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import type {
  PlanChangePackLine,
  SubscriptionFee_FeeCreditUnit,
} from '@/rpc/api/subscriptions/v1/models_pb'

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

const PackChangeRow = ({
  line,
  currency,
  unitNames,
}: {
  line: PlanChangePackLine
  currency: string
  unitNames: (unit: SubscriptionFee_FeeCreditUnit) => CreditUnitNames
}) => {
  const view = packLineView(line, currency, formatPackLineCredit(line, currency, unitNames))
  if (!view) return null
  return (
    <div className="flex justify-between gap-4">
      <div className="min-w-0">
        <div className="text-muted-foreground">{view.label}</div>
        <div className="text-xs text-muted-foreground truncate">
          {line.name}
          {view.detail && ` · ${view.detail}`}
        </div>
      </div>
      <span className="text-foreground shrink-0">{view.value}</span>
    </div>
  )
}

export const StepConfirm = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { previousStep } = useWizard()
  const [state, setState] = useAtom(changePlanAtom)
  const [forceAnnual, setForceAnnual] = useState(false)

  const scheduleMut = useMutation(schedulePlanChange)
  const previewMut = useMutation(previewPlanChange)
  const { parameterizedComponents, ready } = usePlanChangeParameters()

  // Re-fetch preview when apply mode changes to get correct effective_date and proration
  useEffect(() => {
    if (ready && state.targetPlanVersionId && state.subscriptionId) {
      previewMut.mutate(
        {
          subscriptionId: state.subscriptionId,
          newPlanVersionId: state.targetPlanVersionId,
          applyMode: state.applyMode,
          parameterizedComponents,
        },
        {
          onSuccess: data => {
            setState(prev => ({ ...prev, preview: data }))
          },
          onError: error => {
            toast.error(rpcErrorMessage(error, 'Failed to preview the plan change'))
            setState(prev => ({ ...prev, preview: undefined }))
            if (state.applyMode === PlanChangeApplyMode.IMMEDIATE) {
              setState(prev => ({ ...prev, applyMode: PlanChangeApplyMode.END_OF_PERIOD }))
            }
          },
        }
      )
    }
  }, [state.applyMode, ready])

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

  const creditUnits = useCreditUnits()
  const knownUnits = useMemo(
    () => Object.fromEntries((creditUnits ?? []).map(unit => [unit.id, unit])),
    [creditUnits]
  )
  const unitNames = (unit: SubscriptionFee_FeeCreditUnit) => creditUnitNames(unit, knownUnits)
  const isImmediate = state.applyMode === PlanChangeApplyMode.IMMEDIATE
  const immediateAllowed = preview?.immediateAllowed ?? false

  useEffect(() => {
    if (preview && !preview.immediateAllowed && isImmediate) {
      setState(prev => ({ ...prev, applyMode: PlanChangeApplyMode.END_OF_PERIOD }))
    }
  }, [preview, isImmediate, setState])

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
      value === 'immediate' ? PlanChangeApplyMode.IMMEDIATE : PlanChangeApplyMode.END_OF_PERIOD
    setState(prev => ({ ...prev, applyMode: mode }))
  }

  const handleSchedule = async () => {
    if (!state.targetPlanVersionId || !state.subscriptionId) return

    try {
      const result = await scheduleMut.mutateAsync({
        subscriptionId: state.subscriptionId,
        newPlanVersionId: state.targetPlanVersionId,
        applyMode: state.applyMode,
        parameterizedComponents,
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

      await invalidateSubscriptionBilling(queryClient, state.subscriptionId)
      navigate(`../${state.subscriptionId}`, { replace: true })
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'Failed to schedule the plan change'))
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
                <ArrowRight className="h-5 w-5 text-muted-foreground shrink-0" />
                <div className="flex-1 rounded-lg border border-brand/30 bg-brand/5 p-3">
                  <div className="text-xs text-muted-foreground">New Plan</div>
                  <div className="text-sm font-medium text-foreground">{state.targetPlanName}</div>
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
                <div className="flex items-start gap-3">
                  <RadioGroupItem
                    value="immediate"
                    id="mode-immediate"
                    className="mt-0.5"
                    disabled={!immediateAllowed}
                  />
                  <Label
                    htmlFor="mode-immediate"
                    className={
                      immediateAllowed ? 'cursor-pointer' : 'cursor-not-allowed opacity-60'
                    }
                  >
                    <div className="text-sm font-medium">Immediately</div>
                    <div className="text-xs text-muted-foreground">
                      {immediateAllowed || !preview
                        ? 'Apply now with a prorated adjustment invoice.'
                        : 'Not available: this subscription or the new plan has a minimum commitment, which applies for a whole billing period. The change takes effect at the end of the period.'}
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
                        <span className="text-sm font-medium">This is an annual subscription.</span>
                        <p className="text-xs mt-1">
                          Immediate changes to annual plans can result in a large prorated
                          adjustment.
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
                  {preview.proration.packChanges.length > 0 && (
                    <div className="border-t border-border mt-3 pt-3 space-y-2">
                      <div className="text-xs font-medium text-muted-foreground">Credit packs</div>
                      {preview.proration.packChanges.map((line, idx) => (
                        <PackChangeRow
                          key={`${line.kind}-${line.name}-${idx}`}
                          line={line}
                          currency={currency}
                          unitNames={unitNames}
                        />
                      ))}
                    </div>
                  )}
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
          disabled={
            scheduleMut.isPending ||
            previewMut.isPending ||
            !preview ||
            (isImmediate && !immediateAllowed) ||
            (isImmediate && isAnnual && !forceAnnual)
          }
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
