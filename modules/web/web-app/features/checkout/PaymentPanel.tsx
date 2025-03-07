import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { loadStripe } from '@stripe/stripe-js'
import { AlertCircle, Building, CreditCard } from 'lucide-react'
import { useEffect, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum,
} from '@/rpc/api/customers/v1/models_pb'
import { setupIntent } from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'

import { CardBrandLogo } from './components/CardBrandLogo'
import { PaymentForm } from './components/PaymentForm'
import { PaymentMethodSelection, PaymentPanelProps, PaymentState } from './types'

// Inner payment panel component that is wrapped by Elements
const PaymentPanelInner: React.FC<PaymentPanelProps> = ({
  customer,
  paymentMethods,
  onPaymentSubmit,
}) => {
  const stripe = useStripe()
  const elements = useElements()

  const [paymentState, setPaymentState] = useState<PaymentState>(PaymentState.INITIAL)
  const [paymentError, setPaymentError] = useState<string | null>(null)
  const [selectedPaymentMethod, setSelectedPaymentMethod] = useState<PaymentMethodSelection | null>(
    null
  )

  // Initially select default payment method if available
  useEffect(() => {
    if (paymentMethods.length > 0) {
      const defaultMethodId = customer?.currentPaymentMethodId
      const defaultMethod = defaultMethodId
        ? paymentMethods.find(pm => pm.id === defaultMethodId)
        : paymentMethods[0]

      if (defaultMethod) {
        setSelectedPaymentMethod({ type: 'saved', id: defaultMethod.id })
      } else {
        setSelectedPaymentMethod({ type: 'new', methodType: 'card' })
      }
    } else {
      // No saved methods, default to new card
      setSelectedPaymentMethod({ type: 'new', methodType: 'card' })
    }
  }, [paymentMethods, customer])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!selectedPaymentMethod) {
      setPaymentError('Please select a payment method')
      return
    }

    setPaymentState(PaymentState.PROCESSING)
    setPaymentError(null)

    try {
      if (selectedPaymentMethod.type === 'saved') {
        // Use saved payment method
        await onPaymentSubmit(selectedPaymentMethod.id, false)
        setPaymentState(PaymentState.SUCCESS)
      } else if (
        selectedPaymentMethod.type === 'new' &&
        selectedPaymentMethod.methodType === 'card'
      ) {
        // Create new card payment method with Stripe
        if (!stripe || !elements) {
          throw new Error('Stripe has not been initialized')
        }

        // Use confirmSetup instead of createPaymentMethod when using SetupIntent
        const { error, setupIntent } = await stripe.confirmSetup({
          elements,
          confirmParams: {
            return_url: window.location.href,
            payment_method_data: {
              billing_details: {
                name: customer?.name,
                email: customer?.billingEmail,
              },
            },
          },
          redirect: 'if_required',
        })

        if (error) {
          throw new Error(error.message)
        }

        if (setupIntent && setupIntent.payment_method) {
          await onPaymentSubmit(setupIntent.payment_method.toString(), true)
          setPaymentState(PaymentState.SUCCESS)
        } else {
          throw new Error('Payment method creation failed')
        }
      } else if (
        selectedPaymentMethod.type === 'new' &&
        selectedPaymentMethod.methodType === 'bank'
      ) {
        // Bank transfer logic would go here
        // This would typically redirect to a bank authorization page
        throw new Error('Bank payments not implemented yet')
      }
    } catch (err) {
      console.error('Payment error:', err)
      setPaymentError(
        err instanceof Error ? err.message : 'An error occurred during payment processing'
      )
      setPaymentState(PaymentState.ERROR)
    }
  }

  // Render saved payment method card
  const renderSavedPaymentMethod = (method: CustomerPaymentMethod) => {
    const isSelected =
      selectedPaymentMethod?.type === 'saved' && selectedPaymentMethod.id === method.id

    const isCard = method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
    const isDefault = customer?.currentPaymentMethodId === method.id

    return (
      <div
        key={method.id}
        className={`relative flex items-center p-4 border rounded-md mb-2 cursor-pointer ${
          isSelected ? 'border-blue-600 bg-blue-50' : 'border-gray-300'
        }`}
        onClick={() => setSelectedPaymentMethod({ type: 'saved', id: method.id })}
      >
        <div
          className={`w-5 h-5 rounded-full border flex items-center justify-center mr-3 ${
            isSelected ? 'border-blue-600' : 'border-gray-300'
          }`}
        >
          {isSelected && <div className="w-3 h-3 bg-blue-600 rounded-full"></div>}
        </div>

        {isCard ? (
          <>
            <CreditCard size={20} className="mr-3 text-gray-500" />
            <div>
              <div className="font-medium">
                {method.cardBrand} •••• {method.cardLast4}
              </div>
              <div className="text-sm text-gray-500">
                Expires {method.cardExpMonth?.toString().padStart(2, '0')}/
                {method.cardExpYear?.toString().slice(-2)}
              </div>
            </div>
            {method.cardBrand && (
              <div className="ml-auto">
                <CardBrandLogo brand={method.cardBrand} />
              </div>
            )}
          </>
        ) : (
          <>
            <Building size={20} className="mr-3 text-gray-500" />
            <div>
              <div className="font-medium">Bank account</div>
              <div className="text-sm text-gray-500">
                {method.accountNumberHint && `••••${method.accountNumberHint}`}
              </div>
            </div>
          </>
        )}

        {isDefault && (
          <div className="absolute top-1 right-1 bg-blue-100 text-blue-800 text-xs rounded px-1.5 py-0.5">
            Default
          </div>
        )}
      </div>
    )
  }

  return (
    <form onSubmit={handleSubmit} className="max-w-md mx-auto">
      {/* Payment method selection */}
      <div className="mb-8  text-sm">
        <div className="text-sm font-medium mb-4">Pay with</div>

        {/* Saved payment methods */}
        {paymentMethods.length > 0 && (
          <div className="mb-4">
            {paymentMethods.map(method => renderSavedPaymentMethod(method))}
          </div>
        )}

        <div className="mb-2">
          {/* Add new payment method options */}
          {paymentMethods.length > 0 && (
            <>
              <div
                className={`flex items-center p-4 border rounded-md cursor-pointer ${
                  selectedPaymentMethod?.type === 'new' &&
                  selectedPaymentMethod.methodType === 'card'
                    ? 'border-blue-600 bg-blue-50'
                    : 'border-gray-300'
                }`}
                onClick={() => setSelectedPaymentMethod({ type: 'new', methodType: 'card' })}
              >
                <div
                  className={`w-5 h-5 rounded-full border flex items-center justify-center mr-3 ${
                    selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === 'card'
                      ? 'border-blue-600'
                      : 'border-gray-300'
                  }`}
                >
                  {selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === 'card' && (
                      <div className="w-3 h-3 bg-blue-600 rounded-full"></div>
                    )}
                </div>
                <CreditCard size={20} className="mr-3 text-gray-500" />
                <span>Add a credit card</span>
              </div>
            </>
          )}

          {selectedPaymentMethod?.type === 'new' && selectedPaymentMethod.methodType === 'card' && (
            <PaymentForm />
          )}
        </div>

        {/* TODO if bank method enabled */}

        {/* <div>
          <div
            className={`flex items-center p-4 border rounded-md cursor-pointer ${
              selectedPaymentMethod?.type === 'new' && selectedPaymentMethod.methodType === 'bank'
                ? 'border-blue-600 bg-blue-50'
                : 'border-gray-300'
            }`}
            onClick={() => setSelectedPaymentMethod({ type: 'new', methodType: 'bank' })}
          >
            <div
              className={`w-5 h-5 rounded-full border flex items-center justify-center mr-3 ${
                selectedPaymentMethod?.type === 'new' && selectedPaymentMethod.methodType === 'bank'
                  ? 'border-blue-600'
                  : 'border-gray-300'
              }`}
            >
              {selectedPaymentMethod?.type === 'new' &&
                selectedPaymentMethod.methodType === 'bank' && (
                  <div className="w-3 h-3 bg-blue-600 rounded-full"></div>
                )}
            </div>
            <Building size={20} className="mr-3 text-gray-500" />
            <span>Add a bank account</span>
          </div>
        </div> */}
      </div>

      {/* Error message */}
      {paymentError && (
        <div className="mb-4 p-3 bg-red-50 text-red-700 rounded-lg text-sm flex items-start">
          <AlertCircle size={16} className="mr-2 mt-0.5 flex-shrink-0" />
          <span>{paymentError}</span>
        </div>
      )}

      {/* Submit button */}
      <button
        type="submit"
        disabled={paymentState === PaymentState.PROCESSING || !stripe}
        className={`w-full py-3 rounded-lg transition-all font-medium ${
          paymentState === PaymentState.PROCESSING || !stripe
            ? 'bg-gray-400 cursor-not-allowed text-white'
            : 'bg-blue-600 hover:bg-blue-700 text-white'
        }`}
      >
        {paymentState === PaymentState.PROCESSING ? (
          <div className="flex items-center justify-center">
            <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin mr-2"></div>
            Processing...
          </div>
        ) : (
          `Pay and subscribe`
        )}
      </button>

      {/* Security info
      <div className="flex items-center text-xs text-gray-500 mt-6">
        <Lock size={12} className="mr-2" />
        <span>Your payment information is encrypted and secure</span>
      </div> */}

      {/* Footer */}
      <div className="mt-8 flex items-center justify-between text-xs text-muted-foreground">
        <div>Powered by Meteroid</div>
        <div className="flex space-x-4">
          <a
            href="https://meteroid.com/terms"
            className="hover:text-gray-600"
            target="_blank"
            rel="noopener noreferrer"
          >
            Terms
          </a>
          <a
            href="https://meteroid.com/privacy"
            className="hover:text-gray-600"
            target="_blank"
            rel="noopener noreferrer"
          >
            Privacy
          </a>
        </div>
      </div>
    </form>
  )
}

/**
 * Main Payment Panel wrapper component
 * Fetches SetupIntent and initializes Stripe
 */
export const PaymentPanel: React.FC<PaymentPanelProps> = props => {
  // Fetch setup intent to get client secret
  // TODO support direct debit through separate setup as well
  const connection = props.cardConnectionId ?? props.directDebitConnectionId
  const setupIntentQuery = useQuery(
    setupIntent,
    {
      connectionId: connection!,
    },
    { enabled: !!connection }
  )

  // Extract clientSecret and publishableKey from the setupIntent response
  const clientSecret = setupIntentQuery.data?.setupIntent?.intentSecret
  const stripePublishableKey = setupIntentQuery.data?.setupIntent?.providerPublicKey

  // Wait for setup intent to load
  if (setupIntentQuery.isLoading) {
    return <div className="w-full p-6 lg:p-10 text-center">Loading payment options...</div>
  }

  if (setupIntentQuery.isError || !clientSecret || !stripePublishableKey) {
    return (
      <div className="w-full p-6 lg:p-10 text-center text-red-600">
        Unable to initialize payment system. Please try again later.
      </div>
    )
  }

  // Initialize Stripe with the publishable key from the setupIntent
  const stripePromise = loadStripe(stripePublishableKey)

  return (
    <Elements
      stripe={stripePromise}
      options={{
        clientSecret,
        appearance: {
          variables: {
            fontFamily: 'Inter, sans-serif', // TODO pass font
            fontSizeBase: '14px',
            borderRadius: '0.375rem',
            gridRowSpacing: '1rem',
          },
        },
      }}
    >
      <PaymentPanelInner {...props} />
    </Elements>
  )
}
