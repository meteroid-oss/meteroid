import { useMutation } from '@connectrpc/connect-query'
import { Button, Dialog, DialogContent, DialogHeader, DialogTitle } from '@md/ui'
import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { loadStripe } from '@stripe/stripe-js/pure'
import { AlertCircle, Building, CreditCard, ExternalLink } from 'lucide-react'
import { useState } from 'react'

import { PaymentForm } from '@/features/checkout/components/PaymentForm'
import { useQuery } from '@/lib/connectrpc'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { ConnectionTypeEnum } from '@/rpc/portal/shared/v1/models_pb'
import {
  addPaymentMethod,
  setupIntent,
} from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

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
  const hasBoth = hasCard && hasDirectDebit && cardConnectionId !== directDebitConnectionId

  // GoCardless builds its redirect_uri server-side, so this value is informational.
  const activeConnectionId = activeTab === 'card' ? cardConnectionId : directDebitConnectionId

  const returnUrl = typeof window !== 'undefined' ? window.location.href : undefined

  const setupIntentQuery = useQuery(
    setupIntent,
    {
      connectionId: activeConnectionId!,
      connectionType:
        activeTab === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
      returnUrl,
    },
    { enabled: open && !!activeConnectionId }
  )

  const intent = setupIntentQuery.data?.setupIntent
  const intentSecret = intent?.intentSecret
  const provider = intent?.provider
  const stripePublishableKey = intent?.providerPublicKey
  const connectionId = intent?.connectionId
  const isHostedRedirect = provider === ConnectorProviderEnum.GOCARDLESS

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
      <DialogContent className="sm:max-w-[500px] max-h-[calc(100vh-2rem)] overflow-y-auto">
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
            <div className="p-6 text-center text-sm text-gray-600">Loading payment options...</div>
          )}

          {!setupIntentQuery.isLoading &&
            (setupIntentQuery.isError ||
              !intentSecret ||
              !connectionId ||
              (!isHostedRedirect && !stripePublishableKey)) && (
              <div className="p-6 text-center text-sm text-red-600">
                Unable to initialize payment system. Please try again later.
              </div>
            )}

          {/* GoCardless hosted-redirect branch: the backend put the BRF
              authorisation_url in intentSecret. No SDK to mount; we render
              a redirect button. */}
          {intentSecret && connectionId && isHostedRedirect && (
            <div className="p-2">
              <p className="text-sm text-muted-foreground mb-4">
                You&apos;ll be redirected to GoCardless to authorise a direct-debit mandate. Once
                you confirm, you&apos;ll be sent back here.
              </p>
              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={handleCancel}>
                  Cancel
                </Button>
                <a
                  href={intentSecret}
                  className="inline-flex items-center gap-2 px-4 py-2 rounded-md font-medium text-white bg-blue-600 hover:bg-blue-700"
                >
                  <ExternalLink size={14} />
                  Continue to GoCardless
                </a>
              </div>
            </div>
          )}

          {/* Stripe embedded flow */}
          {intentSecret && stripePublishableKey && connectionId && !isHostedRedirect && (
            <Elements
              stripe={loadStripe(stripePublishableKey)}
              options={{
                clientSecret: intentSecret,
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
