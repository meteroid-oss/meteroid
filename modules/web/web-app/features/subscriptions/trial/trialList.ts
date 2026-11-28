import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import type { TrialListFilterSchema } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

export const TRIALS_ENDING_DAYS = 7

export type TrialView = 'all' | 'ending' | 'awaiting_decision' | 'awaiting_payment'

export const TRIAL_VIEWS: { value: TrialView; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'ending', label: `Trials ending · ${TRIALS_ENDING_DAYS} days` },
  { value: 'awaiting_decision', label: 'Awaiting decision' },
  { value: 'awaiting_payment', label: 'Awaiting payment' },
]

export const parseTrialView = (value: string): TrialView =>
  TRIAL_VIEWS.find(view => view.value === value)?.value ?? 'all'

export const trialListFilter = (
  view: TrialView
): MessageInitShape<typeof TrialListFilterSchema> | undefined => {
  switch (view) {
    case 'ending':
      return { kind: { case: 'endingWithinDays', value: TRIALS_ENDING_DAYS } }
    case 'awaiting_decision':
      return { kind: { case: 'awaitingDecision', value: true } }
    case 'awaiting_payment':
      return { kind: { case: 'awaitingPayment', value: true } }
    default:
      return undefined
  }
}

export type TrialPaymentMethod = 'invoice' | 'on_file' | 'none'

/** How a listed trial gets paid; undefined once the trial no longer needs one. */
export const trialPaymentMethod = (
  subscription: Pick<Subscription, 'chargeAutomatically' | 'trial'>
): TrialPaymentMethod | undefined => {
  const hasPaymentMethod = subscription.trial?.hasPaymentMethod
  if (hasPaymentMethod === undefined) return undefined
  if (!subscription.chargeAutomatically) return 'invoice'
  return hasPaymentMethod ? 'on_file' : 'none'
}

/** The method on file as the customer knows it ("Visa •••• 4242"), else "On file". */
export const onFileLabel = (subscription: Pick<Subscription, 'trial'>): string =>
  subscription.trial?.paymentMethod || 'On file'
