import { PaymentElement, useElements, useStripe } from '@stripe/react-stripe-js'

/**
 * Stripe payment form component that renders the PaymentElement
 */
export const PaymentForm = () => {
  const stripe = useStripe()
  const elements = useElements()

  if (!stripe || !elements) {
    return <div className="p-4 text-muted-foreground">Loading...</div>
  }

  return (
    <div className="mt-4 mb-6">
      <div className="py-2 rounded-md mb-3">
        <PaymentElement options={{ layout: 'tabs' }} />
      </div>
    </div>
  )
}
