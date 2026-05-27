import { loadStripe } from '@stripe/stripe-js/pure'

import { PaymentActionRequired } from '@/rpc/api/invoices/v1/models_pb'

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
      const stripe = await loadStripe(publishableKey)
      if (!stripe) throw new Error('Failed to initialize the payment provider')
      const { error } = await stripe.handleNextAction({ clientSecret })
      if (error) throw new Error(error.message ?? 'Card authentication failed')
      return
    }
    case 'redirectToUrl':
      window.location.href = nextAction.action.value
      return
    default:
      return
  }
}
