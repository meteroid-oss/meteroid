import { PaymentElement, useElements, useStripe } from '@stripe/react-stripe-js'
import React from 'react'
import { PaymentFormProps } from '../types'

/**
 * Stripe payment form component that renders the PaymentElement
 */
const PaymentForm: React.FC<PaymentFormProps> = ({}) => {
  const stripe = useStripe()
  const elements = useElements()

  if (!stripe || !elements) {
    return <div className="p-4 text-muted-foreground">Loading...</div>
  }

  // TODO stylize

  return (
    <div className="mt-4 mb-6">
      <div className="py-2 rounded-md mb-3">
        <PaymentElement options={{ layout: 'tabs' }} />
      </div>
    </div>
  )
}

export { PaymentForm }
