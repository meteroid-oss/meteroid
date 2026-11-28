import { Badge, Button } from '@md/ui'
import { useState } from 'react'

import { DetailRow, SidebarSection } from '@/components/layouts/DetailLayout'
import { useTrialPlans } from '@/features/plans/trial/TrialTermsFields'
import { daysLabel } from '@/features/plans/trial/trialTerms'
import { formatDateRange } from '@/features/subscriptions/utils/dates'
import { formatDateOnly } from '@/lib/utils/date'
import { TrialConfig_Status } from '@/rpc/api/subscriptions/v1/models_pb'

import { EndTrialDialog, ExtendTrialDialog } from './TrialActionDialogs'
import { TrialStatusBadge } from './TrialStatusBadge'
import { atEndText, firstChargeText, withoutCardText } from './trialFacts'

import type {
  Subscription,
  SubscriptionFeeBillingPeriod,
  TrialConfig,
} from '@/rpc/api/subscriptions/v1/models_pb'

interface Props {
  subscription: Subscription
  trial: TrialConfig
  isFreePlan: boolean
  /** The periods its fees bill on, for the first charge's "/month". */
  periods: SubscriptionFeeBillingPeriod[]
  /** The customer's payment method, as labelled elsewhere on the page. */
  paymentMethod?: string
  canManage: boolean
  onCancel: () => void
}

/** The subscription's trial: its window, what follows it, and extending or ending it. */
export const SubscriptionTrialSection = ({
  subscription,
  trial,
  isFreePlan,
  periods,
  paymentMethod,
  canManage,
  onCancel,
}: Props) => {
  const { planName } = useTrialPlans()
  const [dialog, setDialog] = useState<'extend' | 'end'>()
  const naming = { isFreePlan, planName, ownName: subscription.planName }
  const active = trial.status === TrialConfig_Status.ACTIVE
  const awaitingDecision = trial.status === TrialConfig_Status.AWAITING_DECISION
  const chargedAutomatically = subscription.chargeAutomatically
  const withoutCard = withoutCardText(trial, { ...naming, chargedAutomatically })

  return (
    <SidebarSection
      title={isFreePlan ? 'Reverse trial' : 'Trial'}
      action={<TrialStatusBadge status={trial.status} />}
    >
      <DetailRow
        title="Trial"
        value={`${
          trial.endsOn
            ? formatDateRange(trial.startsOn, trial.endsOn)
            : formatDateOnly(trial.startsOn)
        } · ${daysLabel(trial.durationDays)}`}
      />
      {trial.endedOn && <DetailRow title="Ended" value={formatDateOnly(trial.endedOn)} />}
      {active && !isFreePlan && trial.firstChargeEstimate && (
        <DetailRow title="Then" value={firstChargeText(trial.firstChargeEstimate, periods)} />
      )}
      {!isFreePlan && (
        <DetailRow
          title="Payment method"
          value={
            !chargedAutomatically ? (
              'Invoice'
            ) : trial.hasPaymentMethod ? (
              (paymentMethod ?? 'On file')
            ) : (
              <Badge variant="warning" size="sm">
                None
              </Badge>
            )
          }
        />
      )}
      <DetailRow title="At the end" value={atEndText(trial, naming)} />
      {withoutCard && <DetailRow title="Without a card" value={withoutCard} />}
      {trial.trialingPlanName && <DetailRow title="Features of" value={trial.trialingPlanName} />}
      {trial.trialOfferName && <DetailRow title="Offer" value={trial.trialOfferName} />}

      {canManage && (active || awaitingDecision) && (
        <div className="flex flex-wrap gap-2 pt-1">
          {awaitingDecision && (
            <Button size="sm" variant="primary" onClick={() => setDialog('end')}>
              Convert now
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={() => setDialog('extend')}>
            {awaitingDecision ? 'Extend' : 'Extend trial'}
          </Button>
          {active && (
            <Button size="sm" variant="outline" onClick={() => setDialog('end')}>
              End trial now
            </Button>
          )}
          {awaitingDecision && (
            <Button
              size="sm"
              variant="ghost"
              className="text-destructive hover:text-destructive"
              onClick={onCancel}
            >
              Cancel
            </Button>
          )}
        </div>
      )}

      {dialog === 'extend' && (
        <ExtendTrialDialog
          subscriptionId={subscription.id}
          endsOn={trial.endsOn}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === 'end' && (
        <EndTrialDialog
          subscriptionId={subscription.id}
          converting={awaitingDecision}
          onClose={() => setDialog(undefined)}
        />
      )}
    </SidebarSection>
  )
}
