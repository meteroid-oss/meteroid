import { ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Button, Dialog, DialogContent, DialogHeader, DialogTitle } from '@md/ui'
import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { AlertCircle, Building, CreditCard, ExternalLink } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'

import { PaymentForm } from '@/features/checkout/components/PaymentForm'
import { buildStripeAppearance } from '@/features/checkout/stripeAppearance'
import { getStripePromise } from '@/features/checkout/stripeClient'
import {
  hostedRedirectHelperText,
  isHostedRedirect,
  useRailTabs,
} from '@/features/checkout/useRailTabs'
import {
  HostedRail,
  consumeHostedReturn,
  hostedReturnErrorMessage,
  hostedReturnSuccessMessage,
  hostedReturnUrl,
  stashHostedDeparture,
} from '@/features/checkout/utils/hostedReturn'
import { connectorDisplayName } from '@/features/payments/providers'
import { useQuery } from '@/lib/connectrpc'
import { usePortalConfig } from '@/pages/portal/experience/PortalThemeProvider'
import { getCustomerPortalOverview } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { ConnectionTypeEnum } from '@/rpc/portal/shared/v1/models_pb'
import {
  addPaymentMethod,
  setupIntent,
} from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

// `processing`: the webhook or sweeper will save the method shortly, so poll instead of
// reporting failure.
const PENDING_POLL_MS = 2000
const PENDING_TIMEOUT_MS = 60 * 1000
// One toast id: pending → outcome updates in place, and StrictMode can't show two.
const HOSTED_TOAST_ID = 'hosted-payment-method-return'

const railNoun = (rail?: HostedRail) =>
  rail === 'directDebit' ? 'direct debit mandate' : rail === 'card' ? 'card' : 'payment method'

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
          <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
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
          className="hover:opacity-90"
          style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
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

  // The dialog renders inside the PortalThemeProvider, so build the Stripe
  // appearance from the resolved portal config.
  const portalConfig = usePortalConfig()
  const stripeAppearance = buildStripeAppearance(portalConfig)

  // Hosted-redirect providers bounce back to this page; the server threads
  // the page URL through as the return target (minus stale provider params).
  const activeConnectionId = activeTab === 'card' ? cardConnectionId : directDebitConnectionId

  const returnUrl = hostedReturnUrl()

  // A hosted authorisation flow redirects back here as a full page load (the
  // dialog is closed). Lazy initializer: the params are stripped on first read.
  const [hostedRet] = useState(() => consumeHostedReturn())
  const [awaitingMethod, setAwaitingMethod] = useState(() => hostedRet?.status === 'processing')

  // Shares the portal overview's cache entry, so polling also refreshes the list behind the dialog.
  const overviewQuery = useQuery(getCustomerPortalOverview, undefined, {
    refetchInterval: awaitingMethod ? PENDING_POLL_MS : false,
  })
  const knownMethods = overviewQuery.data?.overview?.paymentMethods
  const knownMethodIdsRef = useRef<Set<string> | null>(null)

  const onSuccessRef = useRef(onSuccess)
  onSuccessRef.current = onSuccess
  useEffect(() => {
    const ret = hostedRet
    if (!ret) return
    if (ret.status === 'ok') {
      toast.success(hostedReturnSuccessMessage(ret), { id: HOSTED_TOAST_ID })
      onSuccessRef.current?.()
    } else if (ret.status === 'processing') {
      toast.loading(`Confirming your ${railNoun(ret.departure?.rail)}…`, { id: HOSTED_TOAST_ID })
    } else {
      toast.error(hostedReturnErrorMessage(ret), { id: HOSTED_TOAST_ID })
    }
  }, [hostedRet])

  useEffect(() => {
    if (!awaitingMethod || !hostedRet || !knownMethods) return
    if (knownMethodIdsRef.current === null) {
      // The pre-departure snapshot can't contain the new method; seeding from the first poll is racy.
      knownMethodIdsRef.current = new Set(
        hostedRet.departure?.paymentMethodIds ?? knownMethods.map(m => m.id)
      )
    }
    const known = knownMethodIdsRef.current
    if (knownMethods.some(m => !known.has(m.id))) {
      setAwaitingMethod(false)
      toast.success(hostedReturnSuccessMessage(hostedRet), { id: HOSTED_TOAST_ID })
      onSuccessRef.current?.()
    }
  }, [awaitingMethod, hostedRet, knownMethods])

  useEffect(() => {
    if (!awaitingMethod) return
    const timer = setTimeout(() => {
      setAwaitingMethod(false)
      toast.info(
        `Your ${railNoun(hostedRet?.departure?.rail)} is still being confirmed. It will appear here shortly.`,
        { id: HOSTED_TOAST_ID }
      )
    }, PENDING_TIMEOUT_MS)
    return () => clearTimeout(timer)
  }, [awaitingMethod, hostedRet])

  const setupIntentRequest = {
    connectionId: activeConnectionId!,
    connectionType:
      activeTab === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
    returnUrl,
  }
  // Rendering only needs the capabilities; a hosted-payment provider creates nothing until "Continue".
  const setupIntentQuery = useQuery(
    setupIntent,
    { ...setupIntentRequest, descriptorOnly: true },
    { enabled: open && !!activeConnectionId }
  )
  const mintSetupIntent = useMutation(setupIntent)
  const [redirectError, setRedirectError] = useState<string | null>(null)
  useEffect(() => setRedirectError(null), [activeTab, open])

  const intent = setupIntentQuery.data?.setupIntent
  const intentSecret = intent?.intentSecret
  const capabilities = intent?.capabilities
  const { hasBoth } = useRailTabs(cardConnectionId, directDebitConnectionId, capabilities)
  const stripePublishableKey = intent?.providerPublicKey
  const connectionId = intent?.connectionId
  const hostedRedirect = isHostedRedirect(capabilities)
  // The hosted payment is created on click (`descriptorOnly` above).
  const mintsOnClick = !!capabilities?.supportsHostedInvoicePayment
  const hostedProviderLabel = connectorDisplayName(intent?.provider)

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
              (!intentSecret && !mintsOnClick) ||
              !connectionId ||
              (!hostedRedirect && !stripePublishableKey)) && (
              <div className="p-6 text-center text-sm text-red-600">
                Unable to initialize payment system. Please try again later.
              </div>
            )}

          {/* Hosted-redirect branch: the backend put the hosted authorisation URL in
              intentSecret (or mints it on click). No SDK to mount; we render a redirect button. */}
          {(intentSecret || mintsOnClick) && connectionId && hostedRedirect && (
            <div className="p-2">
              <p className="text-sm text-muted-foreground mb-4">
                {hostedRedirectHelperText(
                  hostedProviderLabel,
                  activeTab,
                  "Once you confirm, you'll be sent back here."
                )}
              </p>
              {redirectError && (
                <div className="mb-4 p-3 bg-red-50 text-red-700 rounded-lg text-sm flex items-start">
                  <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
                  <span>{redirectError}</span>
                </div>
              )}
              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={handleCancel}>
                  Cancel
                </Button>
                <Button
                  type="button"
                  disabled={mintSetupIntent.isPending}
                  onClick={async () => {
                    setRedirectError(null)
                    let url = intentSecret
                    if (!url) {
                      try {
                        const res = await mintSetupIntent.mutateAsync(setupIntentRequest)
                        url = res.setupIntent?.intentSecret
                      } catch (err) {
                        // Provider errors are customer-facing: drop the "[code]" prefix.
                        setRedirectError(
                          ConnectError.from(err).rawMessage ||
                            'Unable to start the setup. Please try again.'
                        )
                        return
                      }
                    }
                    if (!url) {
                      setRedirectError('Unable to start the setup. Please try again.')
                      return
                    }
                    stashHostedDeparture({
                      rail: activeTab,
                      paymentMethodIds: knownMethods?.map(m => m.id),
                    })
                    window.location.href = url
                  }}
                >
                  <ExternalLink size={14} className="mr-2" />
                  Continue to {hostedProviderLabel}
                </Button>
              </div>
            </div>
          )}

          {/* Stripe embedded flow */}
          {intentSecret && stripePublishableKey && connectionId && !hostedRedirect && (
            <Elements
              key={intentSecret}
              stripe={getStripePromise(stripePublishableKey)}
              options={{
                clientSecret: intentSecret,
                appearance: stripeAppearance,
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
