import { useMutation } from '@connectrpc/connect-query'
import { Dialog, DialogContent, DialogHeader, DialogTitle, Button } from '@md/ui'
import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { loadStripe } from '@stripe/stripe-js/pure'
import { AlertCircle, CreditCard, Building } from 'lucide-react'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  addPaymentMethod,
  setupIntent,
} from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'
import { ConnectionTypeEnum } from '@/rpc/portal/shared/v1/models_pb'
import { PaymentForm } from '@/features/checkout/components/PaymentForm'

interface AddPaymentMethodDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess?: () => void
  cardConnectionId?: string
  directDebitConnectionId?: string
}

enum PaymentState {
  INITIAL = 'INITIAL',
  PROCESSING = 'PROCESSING',
  SUCCESS = 'SUCCESS',
  ERROR = 'ERROR',
}

// Inner component wrapped by Stripe Elements
const AddPaymentMethodForm: React.FC<{
  activeConnectionId: string
  activeConnectionType: 'card' | 'directDebit'
  onSuccess: () => void
  onCancel: () => void
}> = ({ activeConnectionId, activeConnectionType, onSuccess, onCancel }) => {
  const stripe = useStripe()
  const elements = useElements()

  const [paymentState, setPaymentState] = useState<PaymentState>(PaymentState.INITIAL)
  const [paymentError, setPaymentError] = useState<string | null>(null)

  const addPaymentMethodMutation = useMutation(addPaymentMethod)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    setPaymentState(PaymentState.PROCESSING)
    setPaymentError(null)

    try {
      if (!stripe || !elements) {
        throw new Error('Stripe has not been initialized')
      }

      // Use confirmSetup for both card and direct debit
      const { error, setupIntent } = await stripe.confirmSetup({
        elements,
        confirmParams: {
          return_url: window.location.href,
        },
        redirect: 'if_required',
      })

      if (error) {
        throw new Error(error.message)
      }

      if (setupIntent && setupIntent.payment_method) {
        await addPaymentMethodMutation.mutateAsync({
          connectionId: activeConnectionId,
          externalPaymentMethodId: setupIntent.payment_method.toString(),
        })

        setPaymentState(PaymentState.SUCCESS)
        onSuccess()
      } else {
        throw new Error('Payment method creation failed')
      }
    } catch (err) {
      console.error('Payment method error:', err)
      setPaymentError(
        err instanceof Error ? err.message : 'An error occurred while adding the payment method'
      )
      setPaymentState(PaymentState.ERROR)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="text-sm">
        <div className="flex items-center mb-4 text-gray-700">
          {activeConnectionType === 'card' ? (
            <>
              <CreditCard size={20} className="mr-2 text-gray-500" />
              <span className="font-medium">Add a credit card</span>
            </>
          ) : (
            <>
              <Building size={20} className="mr-2 text-gray-500" />
              <span className="font-medium">Link a bank account</span>
            </>
          )}
        </div>

        <PaymentForm />
      </div>

      {/* Error message */}
      {paymentError && (
        <div className="p-3 bg-red-50 text-red-700 rounded-lg text-sm flex items-start">
          <AlertCircle size={16} className="mr-2 mt-0.5 flex-shrink-0" />
          <span>{paymentError}</span>
        </div>
      )}

      {/* Action buttons */}
      <div className="flex justify-end gap-2 pt-2">
        <Button
          type="button"
          variant="outline"
          onClick={onCancel}
          disabled={paymentState === PaymentState.PROCESSING}
        >
          Cancel
        </Button>
        <Button
          type="submit"
          disabled={paymentState === PaymentState.PROCESSING || !stripe}
          className="bg-blue-600 hover:bg-blue-700"
        >
          {paymentState === PaymentState.PROCESSING ? (
            <div className="flex items-center">
              <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin mr-2"></div>
              Adding...
            </div>
          ) : (
            'Add payment method'
          )}
        </Button>
      </div>
    </form>
  )
}

export const AddPaymentMethodDialog: React.FC<AddPaymentMethodDialogProps> = ({
  open,
  onOpenChange,
  onSuccess,
  cardConnectionId,
  directDebitConnectionId,
}) => {
  const [activeTab, setActiveTab] = useState<'card' | 'directDebit'>(
    cardConnectionId ? 'card' : 'directDebit'
  )

  const hasCard = !!cardConnectionId
  const hasDirectDebit = !!directDebitConnectionId
  const hasBoth = hasCard && hasDirectDebit

  // Fetch setup intent for the active connection
  const activeConnectionId =
    activeTab === 'card' ? cardConnectionId : directDebitConnectionId

  const setupIntentQuery = useQuery(
    setupIntent,
    {
      connectionId: activeConnectionId!,
      connectionType:
        activeTab === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
    },
    { enabled: open && !!activeConnectionId }
  )

  // Extract clientSecret and publishableKey from the setupIntent response
  const clientSecret = setupIntentQuery.data?.setupIntent?.intentSecret
  const stripePublishableKey = setupIntentQuery.data?.setupIntent?.providerPublicKey
  const connectionId = setupIntentQuery.data?.setupIntent?.connectionId

  const handleSuccess = () => {
    onOpenChange(false)
    if (onSuccess) {
      onSuccess()
    }
  }

  const handleCancel = () => {
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>Add payment method</DialogTitle>
        </DialogHeader>

        <div className="mt-4">
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

          {/* Loading/Error states */}
          {setupIntentQuery.isLoading && (
            <div className="p-6 text-center text-sm text-gray-600">
              Loading payment options...
            </div>
          )}

          {(setupIntentQuery.isError || !clientSecret || !stripePublishableKey || !connectionId) && (
            <div className="p-6 text-center text-sm text-red-600">
              Unable to initialize payment system. Please try again later.
            </div>
          )}

          {/* Payment form */}
          {clientSecret && stripePublishableKey && connectionId && (
            <Elements
              stripe={loadStripe(stripePublishableKey)}
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
              <AddPaymentMethodForm
                activeConnectionId={connectionId}
                activeConnectionType={activeTab}
                onSuccess={handleSuccess}
                onCancel={handleCancel}
              />
            </Elements>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
