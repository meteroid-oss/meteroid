import { useMutation } from '@connectrpc/connect-query'
import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { loadStripe } from '@stripe/stripe-js/pure' // prevents calls to stripe until used
import { AlertCircle, Building, CreditCard } from 'lucide-react'
import { useEffect, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum,
} from '@/rpc/api/customers/v1/models_pb'
import { ConnectionTypeEnum } from '@/rpc/portal/shared/v1/models_pb'
import {
  addPaymentMethod,
  setupIntent,
} from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

import { CardBrandLogo } from './components/CardBrandLogo'
import { PaymentForm } from './components/PaymentForm'
import { PaymentMethodSelection, PaymentPanelProps, PaymentState } from './types'

// Inner payment panel component that is wrapped by Elements
const PaymentPanelInner: React.FC<
  PaymentPanelProps & {
    activeConnectionId: string
    activeConnectionType: 'card' | 'directDebit'
  }
> = ({ customer, paymentMethods, onPaymentSubmit, activeConnectionId, activeConnectionType }) => {
  const stripe = useStripe()
  const elements = useElements()

  const [paymentState, setPaymentState] = useState<PaymentState>(PaymentState.INITIAL)
  const [paymentError, setPaymentError] = useState<string | null>(null)
  const [selectedPaymentMethod, setSelectedPaymentMethod] = useState<PaymentMethodSelection | null>(
    null
  )

  const addPaymentMethodMutation = useMutation(addPaymentMethod)

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
        setSelectedPaymentMethod({ type: 'new', methodType: activeConnectionType })
      }
    } else {
      // No saved methods, default to active connection type
      setSelectedPaymentMethod({ type: 'new', methodType: activeConnectionType })
    }
  }, [paymentMethods, customer, activeConnectionType])

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
        await onPaymentSubmit(selectedPaymentMethod.id)
        setPaymentState(PaymentState.SUCCESS)
      } else if (
        selectedPaymentMethod.type === 'new' &&
        (selectedPaymentMethod.methodType === 'card' ||
          selectedPaymentMethod.methodType === 'directDebit')
      ) {
        // Create new payment method with Stripe (card or direct debit)
        if (!stripe || !elements) {
          throw new Error('Stripe has not been initialized')
        }

        // Use confirmSetup for both card and direct debit
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
          const res = await addPaymentMethodMutation.mutateAsync({
            connectionId: activeConnectionId,
            externalPaymentMethodId: setupIntent.payment_method.toString(),
          })

          if (!res.paymentMethod?.id) {
            throw new Error('Payment method creation failed. No id returned')
          }

          await onPaymentSubmit(res.paymentMethod.id)
          setPaymentState(PaymentState.SUCCESS)
        } else {
          throw new Error('Payment method creation failed')
        }
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
        className={`flex items-center p-2 border rounded-md mb-2 cursor-pointer ${
          isSelected ? 'border-blue-600 bg-blue-50' : 'border-gray-300'
        }`}
        onClick={() => setSelectedPaymentMethod({ type: 'saved', id: method.id })}
      >
        <div
          className={`w-3 h-3 rounded-full border flex items-center justify-center mr-3 ${
            isSelected ? 'border-blue-600' : 'border-gray-300'
          }`}
        >
          {isSelected && <div className="w-2 h-2 bg-blue-600 rounded-full"></div>}
        </div>

        {isCard ? (
          <>
            <CreditCard size={20} className="mr-3 text-gray-500 flex-shrink-0" />
            <div className="min-w-0">
              <div className="font-medium text-sm truncate">
                {method.cardBrand} •••• {method.cardLast4}
              </div>
              <div className="text-xs text-gray-500">
                Expires {method.cardExpMonth?.toString().padStart(2, '0')}/
                {method.cardExpYear?.toString().slice(-2)}
              </div>
            </div>
            <div className="ml-auto flex items-center gap-2 flex-shrink-0">
              {isDefault && (
                <div className="bg-gray-100 text-gray-500 text-xs font-medium rounded px-2 py-1">
                  Default
                </div>
              )}
              {method.cardBrand && <CardBrandLogo brand={method.cardBrand} />}
            </div>
          </>
        ) : (
          <>
            <Building size={20} className="mr-3 text-gray-500 flex-shrink-0" />
            <div className="min-w-0">
              <div className="font-medium truncate">Bank account</div>
              <div className="text-xs text-gray-500">
                {method.accountNumberHint && `••••${method.accountNumberHint}`}
              </div>
            </div>
            {isDefault && (
              <div className="ml-auto bg-gray-100 text-gray-500 text-xs font-medium rounded px-2 py-1 flex-shrink-0">
                Default
              </div>
            )}
          </>
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
            {paymentMethods
              // .filter(pm => paymentMethodMatches(pm.paymentMethodType, activeConnectionType))
              .map(method => renderSavedPaymentMethod(method))}
          </div>
        )}

        <div className="mb-2">
          {/* Add new payment method options */}
          {paymentMethods.length > 0 && (
            <>
              <div
                className={`flex items-center p-4 border rounded-md cursor-pointer ${
                  selectedPaymentMethod?.type === 'new' &&
                  selectedPaymentMethod.methodType === activeConnectionType
                    ? 'border-blue-600 bg-blue-50'
                    : 'border-gray-300'
                }`}
                onClick={() =>
                  setSelectedPaymentMethod({ type: 'new', methodType: activeConnectionType })
                }
              >
                <div
                  className={`w-3 h-3 rounded-full border flex items-center justify-center mr-3 ${
                    selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === activeConnectionType
                      ? 'border-blue-600'
                      : 'border-gray-300'
                  }`}
                >
                  {selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === activeConnectionType && (
                      <div className="w-2 h-2 bg-blue-600 rounded-full"></div>
                    )}
                </div>
                {activeConnectionType === 'card' ? (
                  <>
                    <CreditCard size={20} className="mr-3 text-gray-500" />
                    <span>Add a credit card</span>
                  </>
                ) : (
                  <>
                    <Building size={20} className="mr-3 text-gray-500" />
                    <span>Link a bank account</span>
                  </>
                )}
              </div>
            </>
          )}

          {selectedPaymentMethod?.type === 'new' &&
            selectedPaymentMethod.methodType === activeConnectionType && <PaymentForm />}
        </div>
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
 * Supports both card and direct debit payment methods
 */
export const PaymentPanel: React.FC<PaymentPanelProps> = props => {
  const [activeTab, setActiveTab] = useState<'card' | 'directDebit'>(
    props.cardConnectionId ? 'card' : 'directDebit'
  )

  const hasCard = !!props.cardConnectionId
  const hasDirectDebit = !!props.directDebitConnectionId
  const hasBoth =
    hasCard && hasDirectDebit && props.cardConnectionId !== props.directDebitConnectionId

  // Fetch setup intent for the active connection
  const activeConnectionId =
    activeTab === 'card' ? props.cardConnectionId : props.directDebitConnectionId

  const setupIntentQuery = useQuery(
    setupIntent,
    {
      connectionId: activeConnectionId!,
      connectionType:
        activeTab === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
    },
    { enabled: !!activeConnectionId }
  )

  // Extract clientSecret and publishableKey from the setupIntent response
  const clientSecret = setupIntentQuery.data?.setupIntent?.intentSecret
  const stripePublishableKey = setupIntentQuery.data?.setupIntent?.providerPublicKey
  const connectionId = setupIntentQuery.data?.setupIntent?.connectionId

  // Wait for setup intent to load
  if (setupIntentQuery.isLoading) {
    return <div className="w-full p-6 lg:p-10 text-center">Loading payment options...</div>
  }

  if (setupIntentQuery.isError || !clientSecret || !stripePublishableKey || !connectionId) {
    console.log(
      `setupIntent error: ${
        setupIntentQuery.isError
          ? setupIntentQuery.error
          : `Missing ${!clientSecret ? 'clientSecret ' : ''}${!stripePublishableKey ? 'stripePublishableKey ' : ''}${!connectionId ? 'connectionId' : ''}`
      } `
    )

    return (
      <div className="w-full p-6 lg:p-10 text-center text-red-600">
        Unable to initialize payment system. Please try again later.
      </div>
    )
  }

  // Initialize Stripe with the publishable key from the setupIntent
  const stripePromise = loadStripe(stripePublishableKey)

  return (
    <div>
      {/* Tabs for card/direct debit if both are available */}
      {hasBoth && (
        <div className="flex border-b border-gray-200 mb-6">
          <button
            type="button"
            className={`flex-1 py-3 px-4 text-sm font-medium transition-colors ${
              activeTab === 'card'
                ? 'border-b-2 border-blue-600 text-blue-600'
                : 'text-gray-500 hover:text-gray-700'
            }`}
            onClick={() => setActiveTab('card')}
          >
            <div className="flex items-center justify-center">
              <CreditCard size={16} className="mr-2" />
              Card
            </div>
          </button>
          <button
            type="button"
            className={`flex-1 py-3 px-4 text-sm font-medium transition-colors ${
              activeTab === 'directDebit'
                ? 'border-b-2 border-blue-600 text-blue-600'
                : 'text-gray-500 hover:text-gray-700'
            }`}
            onClick={() => setActiveTab('directDebit')}
          >
            <div className="flex items-center justify-center">
              <Building size={16} className="mr-2" />
              Direct Debit
            </div>
          </button>
        </div>
      )}

      {/* Payment form */}
      <Elements
        stripe={stripePromise}
        options={{
          clientSecret,

          appearance: {
            variables: {
              fontFamily: 'Inter, sans-serif',
              fontSizeBase: '14px',
              borderRadius: '0.375rem',
              gridRowSpacing: '1rem',
            },
          },
        }}
      >
        <PaymentPanelInner
          {...props}
          activeConnectionId={connectionId}
          activeConnectionType={activeTab}
        />
      </Elements>
    </div>
  )
}
