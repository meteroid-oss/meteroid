import { PaymentActionRequired } from '@/rpc/api/invoices/v1/models_pb'

import { getStripePromise } from '../stripeClient'

/**
 * Complete a 3DS/SCA step returned by a confirm response. Resolves on success;
 * throws on authentication failure so the caller surfaces it. A redirect action
 * navigates away and never resolves.
 */
export const completeNextAction = async (
  nextAction: PaymentActionRequired | undefined
): Promise<void> => {
  if (!nextAction) return

  switch (nextAction.action.case) {
    case 'useSdk': {
      const { publishableKey, clientSecret } = nextAction.action.value
      const stripe = await getStripePromise(publishableKey)
      if (!stripe) throw new Error('Failed to initialize the payment provider')
      const { error } = await stripe.handleNextAction({ clientSecret })
      if (error) throw new Error(error.message ?? 'Card authentication failed')
      return
    }
    case 'redirectToUrl':
      window.location.href = nextAction.action.value
      // Navigation is in-flight but not instantaneous. Never resolve, so the
      // caller can't push a premature success page (which would also be what
      // Back from the provider page lands on).
      await new Promise<never>(() => {})
      // Unreachable — the promise above never settles.
      return
    default:
      // A next action we don't know how to complete must not silently succeed.
      throw new Error(`Unsupported payment next action: ${nextAction.action.case ?? 'none'}`)
  }
}
