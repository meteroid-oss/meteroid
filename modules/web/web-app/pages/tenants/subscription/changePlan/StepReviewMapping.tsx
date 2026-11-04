import { useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Skeleton } from '@md/ui'
import { cn } from '@ui/lib'
import { useAtom } from 'jotai'
import { ArrowRight, Minus, Plus } from 'lucide-react'
import { useEffect } from 'react'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { formatSubscriptionFeeCompact } from '@/features/subscriptions/utils/fees'
import { ComponentChoiceControls } from '@/pages/tenants/subscription/changePlan/ComponentChoiceControls'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { usePlanChangeParameters } from '@/pages/tenants/subscription/changePlan/usePlanChangeParameters'
import { SubscriptionFee, SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { previewPlanChange } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PlanChangeAddedComponent } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

const formatPeriod = (period: SubscriptionFeeBillingPeriod): string => {
  const map: Record<number, string> = {
    [SubscriptionFeeBillingPeriod.ONE_TIME]: 'One Time',
    [SubscriptionFeeBillingPeriod.MONTHLY]: 'Monthly',
    [SubscriptionFeeBillingPeriod.QUARTERLY]: 'Quarterly',
    [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'Semiannual',
    [SubscriptionFeeBillingPeriod.YEARLY]: 'Yearly',
  }
  return map[period] ?? ''
}

const FeeLabel = ({
  fee,
  period,
  currency,
}: {
  fee?: SubscriptionFee
  period?: SubscriptionFeeBillingPeriod
  currency: string
}) => {
  const creditUnits = useCreditUnits()
  return (
    <span className="text-xs text-muted-foreground">
      {formatSubscriptionFeeCompact(fee as SubscriptionFee | undefined, currency, creditUnits)}
      {period !== undefined && <span className="ml-1.5 opacity-70">{formatPeriod(period)}</span>}
    </span>
  )
}

const DirectionBadge = ({ direction }: { direction: string }) => {
  switch (direction) {
    case 'upgrade':
      return (
        <Badge variant="default" size="sm" className="bg-brand text-brand-foreground">
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
  const { addedTargets, configurable, parameterizedComponents, parametersKey, choose, ready } =
    usePlanChangeParameters()

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

  // Preview rows carry no component id: the added ones follow the target plan's order.
  const configurableFor = (added: PlanChangeAddedComponent, idx: number) => {
    const target =
      addedTargets[idx]?.name === added.name
        ? addedTargets[idx]
        : addedTargets.find(t => t.name === added.name)
    return target && configurable.find(c => c.component.id === target.id)
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

        {/* Table view */}
        <div
          className={cn(
            'bg-card rounded-lg shadow-sm overflow-hidden transition-opacity',
            previewMut.isPending && 'opacity-60'
          )}
        >
          <table className="w-full">
            <thead className="bg-muted/40">
              <tr>
                <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  Current
                </th>
                <th className="px-2 py-3 w-8" />
                <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  New
                </th>
                <th className="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  Status
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border">
              {preview.matched.map((match, idx) => (
                <tr key={`matched-${idx}`}>
                  <td className="px-4 py-3">
                    <div className="text-sm text-foreground">{match.currentName}</div>
                    <FeeLabel
                      fee={match.currentFee}
                      period={match.currentPeriod}
                      currency={currency}
                    />
                  </td>
                  <td className="px-2 py-3 text-center">
                    <ArrowRight className="h-4 w-4 text-muted-foreground inline-block" />
                  </td>
                  <td className="px-4 py-3">
                    <div className="text-sm font-medium text-foreground">{match.newName}</div>
                    <FeeLabel fee={match.newFee} period={match.newPeriod} currency={currency} />
                  </td>
                  <td className="px-4 py-3 text-right">
                    <Badge variant="secondary" size="sm">
                      Matched
                    </Badge>
                  </td>
                </tr>
              ))}

              {preview.added.map((added, idx) => {
                const choices = configurableFor(added, idx)
                return (
                  <tr key={`added-${idx}`}>
                    <td className="px-4 py-3 text-sm text-muted-foreground italic">--</td>
                    <td className="px-2 py-3 text-center">
                      <Plus className="h-4 w-4 text-brand inline-block" />
                    </td>
                    <td className="px-4 py-3">
                      <div className="text-sm font-medium text-foreground">{added.name}</div>
                      <FeeLabel fee={added.fee} period={added.period} currency={currency} />
                      {choices && (
                        <ComponentChoiceControls
                          configurable={choices}
                          currency={currency}
                          slotUnit={
                            added.fee?.fee.case === 'slot' ? added.fee.fee.value.unit : undefined
                          }
                          onChange={choice => choose(choices.component.id, choice)}
                        />
                      )}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <Badge variant="outline" size="sm" className="text-brand border-brand/30">
                        Added
                      </Badge>
                    </td>
                  </tr>
                )
              })}

              {preview.removed.map((removed, idx) => (
                <tr key={`removed-${idx}`} className="bg-destructive/5">
                  <td className="px-4 py-3">
                    <div className="text-sm text-foreground line-through">{removed.name}</div>
                    <FeeLabel
                      fee={removed.currentFee}
                      period={removed.currentPeriod}
                      currency={currency}
                    />
                  </td>
                  <td className="px-2 py-3 text-center">
                    <Minus className="h-4 w-4 text-destructive inline-block" />
                  </td>
                  <td className="px-4 py-3 text-sm text-muted-foreground italic">--</td>
                  <td className="px-4 py-3 text-right">
                    <Badge variant="destructive" size="sm">
                      Removed
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          {preview.matched.length === 0 &&
            preview.added.length === 0 &&
            preview.removed.length === 0 && (
              <div className="p-8 text-center text-sm text-muted-foreground">
                No component changes detected between the two plans.
              </div>
            )}
        </div>
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
