import { CardElement } from '@stripe/react-stripe-js'
import { FC } from 'react'

const CARD_ELEMENT_OPTIONS = {
  style: {
    base: {
      color: '#32325d',
      fontFamily: '"Helvetica Neue", Helvetica, sans-serif',
      fontSmoothing: 'antialiased',
      fontSize: '16px',
      '::placeholder': {
        color: '#aab7c4',
      },
    },
    invalid: {
      color: '#fa755a',
      iconColor: '#fa755a',
    },
  },
}

const StripeCardForm: FC = () => {
  return (
    <div className="bg-white p-3 rounded border border-gray-200 mb-3">
      <CardElement options={CARD_ELEMENT_OPTIONS} />
    </div>
  )
}

export default StripeCardForm
