import { useMutation } from '@connectrpc/connect-query'
import { Elements, useElements, useStripe } from '@stripe/react-stripe-js'
import { AlertCircle, Building, CreditCard, ExternalLink } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
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
import { buildStripeAppearance } from './stripeAppearance'
import { getStripePromise } from './stripeClient'
import { PaymentMethodSelection, PaymentPanelProps, PaymentState } from './types'
import { gocardlessReturnUrl, stashGocardlessPreAttempt } from './utils/gocardlessReturn'

/** Which tab a saved payment method belongs under. Everything that isn't a
 *  card (SEPA/ACH/BACS/bank account) is a direct-debit method. */
const tabForMethodType = (
  t: CustomerPaymentMethod_PaymentMethodTypeEnum
): 'card' | 'directDebit' =>
  t === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD ? 'card' : 'directDebit'

/** One selectable saved-method row (card or bank account). Shared by the Stripe
 *  panel and the GoCardless saved-mandate panel. */
const SavedMethodRow: React.FC<{
  method: CustomerPaymentMethod
  selected: boolean
  isDefault: boolean
  onSelect: () => void
}> = ({ method, selected, isDefault, onSelect }) => {
  const isCard = method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
  return (
    <div
      className="flex items-center p-2 border rounded-md mb-2 cursor-pointer"
      style={{
        borderColor: selected ? 'var(--mtp-accent)' : 'var(--mtp-border)',
        background: selected ? 'var(--mtp-accent-weak)' : 'transparent',
      }}
      onClick={onSelect}
    >
      <div
        className="w-3 h-3 rounded-full border flex items-center justify-center mr-3"
        style={{ borderColor: selected ? 'var(--mtp-accent)' : 'var(--mtp-border-2)' }}
      >
        {selected && (
          <div className="w-2 h-2 rounded-full" style={{ background: 'var(--mtp-accent)' }}></div>
        )}
      </div>

      {isCard ? (
        <>
          <CreditCard size={20} className="mr-3 shrink-0" style={{ color: 'var(--mtp-text-2)' }} />
          <div className="min-w-0">
            <div className="font-medium text-sm truncate">
              {method.cardBrand} •••• {method.cardLast4}
            </div>
            <div className="text-xs" style={{ color: 'var(--mtp-text-2)' }}>
              Expires {method.cardExpMonth?.toString().padStart(2, '0')}/
              {method.cardExpYear?.toString().slice(-2)}
            </div>
          </div>
          <div className="ml-auto flex items-center gap-2 shrink-0">
            {isDefault && (
              <div
                className="text-xs font-medium rounded px-2 py-1"
                style={{ background: 'var(--mtp-surface-2)', color: 'var(--mtp-text-2)' }}
              >
                Default
              </div>
            )}
            {method.cardBrand && <CardBrandLogo brand={method.cardBrand} />}
          </div>
        </>
      ) : (
        <>
          <Building size={20} className="mr-3 shrink-0" style={{ color: 'var(--mtp-text-2)' }} />
          <div className="min-w-0">
            <div className="font-medium truncate">Bank account</div>
            {method.accountNumberHint && (
              <div className="text-xs" style={{ color: 'var(--mtp-text-2)' }}>
                ••••{method.accountNumberHint}
              </div>
            )}
          </div>
          {isDefault && (
            <div
              className="ml-auto text-xs font-medium rounded px-2 py-1 shrink-0"
              style={{ background: 'var(--mtp-surface-2)', color: 'var(--mtp-text-2)' }}
            >
              Default
            </div>
          )}
        </>
      )}
    </div>
  )
}

/** GoCardless direct-debit panel when the customer already has a mandate: a
 *  mandate is reusable, so we charge it off-session (no second hosted redirect).
 *  Stripe-free — never wrapped in <Elements>. `addNewUrl` opens the hosted flow
 *  only when the customer wants to set up an additional bank account. */
const SavedMandatePanel: React.FC<{
  methods: CustomerPaymentMethod[]
  customer?: PaymentPanelProps['customer']
  onPaymentSubmit: (paymentMethodId: string) => Promise<void>
  addNewUrl: string
  /** Run synchronously before the hosted "add new bank account" flow navigates away. */
  onBeforeRedirect?: () => void
}> = ({ methods, customer, onPaymentSubmit, addNewUrl, onBeforeRedirect }) => {
  const [selectedId, setSelectedId] = useState<string>(
    methods.find(m => m.id === customer?.currentPaymentMethodId)?.id ?? methods[0]?.id ?? ''
  )
  const [state, setState] = useState<PaymentState>(PaymentState.INITIAL)
  const [error, setError] = useState<string | null>(null)

  const handlePay = async () => {
    if (!selectedId) return
    setState(PaymentState.PROCESSING)
    setError(null)
    try {
      await onPaymentSubmit(selectedId)
      setState(PaymentState.SUCCESS)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'An error occurred during payment processing')
      setState(PaymentState.ERROR)
    }
  }

  return (
    <div className="max-w-md mx-auto text-sm">
      <div className="text-sm font-medium mb-4">Pay with</div>
      <div className="mb-4">
        {methods.map(method => (
          <SavedMethodRow
            key={method.id}
            method={method}
            selected={selectedId === method.id}
            isDefault={customer?.currentPaymentMethodId === method.id}
            onSelect={() => setSelectedId(method.id)}
          />
        ))}
      </div>

      <a
        href={addNewUrl}
        onClick={onBeforeRedirect}
        className="flex items-center p-4 mb-2 border rounded-md hover:opacity-90"
        style={{ borderColor: 'var(--mtp-border)' }}
      >
        <Building size={20} className="mr-3 shrink-0" style={{ color: 'var(--mtp-text-2)' }} />
        <span>Set up a new bank account</span>
        <ExternalLink
          size={14}
          className="ml-auto shrink-0"
          style={{ color: 'var(--mtp-text-2)' }}
        />
      </a>

      {error && (
        <div
          className="mb-4 p-3 rounded-lg text-sm flex items-start"
          style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
        >
          <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <button
        type="button"
        onClick={handlePay}
        disabled={state === PaymentState.PROCESSING || !selectedId}
        className="w-full mt-2 py-3 rounded-lg transition-all font-medium disabled:cursor-not-allowed disabled:opacity-60 hover:opacity-90"
        style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
      >
        {state === PaymentState.PROCESSING ? (
          <div className="flex items-center justify-center">
            <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin mr-2"></div>
            Processing...
          </div>
        ) : (
          'Pay'
        )}
      </button>
    </div>
  )
}

const PaymentPanelInner: React.FC<
  PaymentPanelProps & {
    activeConnectionId: string
    activeConnectionType: 'card' | 'directDebit'
  }
> = ({
  customer,
  paymentMethods,
  onPaymentSubmit,
  onPaymentMethodAttached,
  activeConnectionId,
  activeConnectionType,
}) => {
  const stripe = useStripe()
  const elements = useElements()

  const [paymentState, setPaymentState] = useState<PaymentState>(PaymentState.INITIAL)
  const [paymentError, setPaymentError] = useState<string | null>(null)
  const [selectedPaymentMethod, setSelectedPaymentMethod] = useState<PaymentMethodSelection | null>(
    null
  )
  // A SetupIntent can be confirmed exactly once. If a later step fails (e.g. the
  // customer dismisses the 3DS challenge), we must not re-run confirmSetup on
  // the now-consumed intent; remember the attached method and reuse it on retry.
  const [attachedPaymentMethodId, setAttachedPaymentMethodId] = useState<string | null>(null)

  const addPaymentMethodMutation = useMutation(addPaymentMethod)

  // Only saved methods that belong under the active tab (a bank account under
  // Direct Debit, a card under Card) — never show a bank account on the Card tab.
  const savedMethodsForTab = paymentMethods.filter(
    m => tabForMethodType(m.paymentMethodType) === activeConnectionType
  )

  useEffect(() => {
    if (savedMethodsForTab.length > 0) {
      // Prefer the customer's default method if it's usable on this tab, else the
      // first method that is; only then fall back to "add a new one".
      const defaultMethodId = customer?.currentPaymentMethodId
      const defaultMethod =
        savedMethodsForTab.find(pm => pm.id === defaultMethodId) ?? savedMethodsForTab[0]
      setSelectedPaymentMethod({ type: 'saved', id: defaultMethod.id })
    } else {
      setSelectedPaymentMethod({ type: 'new', methodType: activeConnectionType })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
        await onPaymentSubmit(selectedPaymentMethod.id)
        setPaymentState(PaymentState.SUCCESS)
      } else if (
        selectedPaymentMethod.type === 'new' &&
        (selectedPaymentMethod.methodType === 'card' ||
          selectedPaymentMethod.methodType === 'directDebit')
      ) {
        // On a retry after a partial failure the intent was already confirmed
        // and the method attached — go straight to payment, don't re-confirm.
        let paymentMethodId = attachedPaymentMethodId

        if (!paymentMethodId) {
          if (!stripe || !elements) {
            throw new Error('Stripe has not been initialized')
          }

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

          if (!setupIntent || !setupIntent.payment_method) {
            throw new Error('Payment method creation failed')
          }

          const res = await addPaymentMethodMutation.mutateAsync({
            connectionId: activeConnectionId,
            externalPaymentMethodId: setupIntent.payment_method.toString(),
          })

          if (!res.paymentMethod?.id) {
            throw new Error('Payment method creation failed. No id returned')
          }

          paymentMethodId = res.paymentMethod.id
          setAttachedPaymentMethodId(paymentMethodId)
          // Let the parent refresh its payment-method list so the newly
          // attached method is present/selectable.
          onPaymentMethodAttached?.()
        }

        await onPaymentSubmit(paymentMethodId)
        setPaymentState(PaymentState.SUCCESS)
      }
    } catch (err) {
      console.error('Payment error:', err)
      setPaymentError(
        err instanceof Error ? err.message : 'An error occurred during payment processing'
      )
      setPaymentState(PaymentState.ERROR)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="max-w-md mx-auto">
      {/* Payment method selection */}
      <div className="mb-8  text-sm">
        <div className="text-sm font-medium mb-4">Pay with</div>

        {/* Saved payment methods usable on this tab */}
        {savedMethodsForTab.length > 0 && (
          <div className="mb-4">
            {savedMethodsForTab.map(method => (
              <SavedMethodRow
                key={method.id}
                method={method}
                selected={
                  selectedPaymentMethod?.type === 'saved' && selectedPaymentMethod.id === method.id
                }
                isDefault={customer?.currentPaymentMethodId === method.id}
                onSelect={() => setSelectedPaymentMethod({ type: 'saved', id: method.id })}
              />
            ))}
          </div>
        )}

        <div className="mb-2">
          {/* Add new payment method options */}
          {savedMethodsForTab.length > 0 && (
            <>
              <div
                className="flex items-center p-4 border rounded-md cursor-pointer"
                style={{
                  borderColor:
                    selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === activeConnectionType
                      ? 'var(--mtp-accent)'
                      : 'var(--mtp-border)',
                  background:
                    selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === activeConnectionType
                      ? 'var(--mtp-accent-weak)'
                      : 'transparent',
                }}
                onClick={() =>
                  setSelectedPaymentMethod({ type: 'new', methodType: activeConnectionType })
                }
              >
                <div
                  className="w-3 h-3 rounded-full border flex items-center justify-center mr-3"
                  style={{
                    borderColor:
                      selectedPaymentMethod?.type === 'new' &&
                      selectedPaymentMethod.methodType === activeConnectionType
                        ? 'var(--mtp-accent)'
                        : 'var(--mtp-border-2)',
                  }}
                >
                  {selectedPaymentMethod?.type === 'new' &&
                    selectedPaymentMethod.methodType === activeConnectionType && (
                      <div
                        className="w-2 h-2 rounded-full"
                        style={{ background: 'var(--mtp-accent)' }}
                      ></div>
                    )}
                </div>
                {activeConnectionType === 'card' ? (
                  <>
                    <CreditCard size={20} className="mr-3" style={{ color: 'var(--mtp-text-2)' }} />
                    <span>Add a credit card</span>
                  </>
                ) : (
                  <>
                    <Building size={20} className="mr-3" style={{ color: 'var(--mtp-text-2)' }} />
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
        <div
          className="mb-4 p-3 rounded-lg text-sm flex items-start"
          style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
        >
          <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
          <span>{paymentError}</span>
        </div>
      )}

      {/* Submit button */}
      <button
        type="submit"
        disabled={paymentState === PaymentState.PROCESSING || !stripe}
        className="w-full py-3 rounded-lg transition-all font-medium disabled:cursor-not-allowed disabled:opacity-60 hover:opacity-90"
        style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
      >
        {paymentState === PaymentState.PROCESSING ? (
          <div className="flex items-center justify-center">
            <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin mr-2"></div>
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
      <div
        className="mt-8 flex items-center justify-between text-xs"
        style={{ color: 'var(--mtp-text-2)' }}
      >
        <div>Powered by Meteroid</div>
        <div className="flex space-x-4">
          <a
            href="https://meteroid.com/terms"
            className="hover:opacity-80"
            target="_blank"
            rel="noopener noreferrer"
          >
            Terms
          </a>
          <a
            href="https://meteroid.com/privacy"
            className="hover:opacity-80"
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
  // Open on the tab that holds the customer's default/first saved method (so a
  // returning direct-debit customer lands on Direct Debit, not Card) — but only
  // if that rail is actually available here; otherwise fall back to whichever
  // rail is configured.
  const preferredMethod =
    props.paymentMethods.find(m => m.id === props.customer?.currentPaymentMethodId) ??
    props.paymentMethods[0]
  const preferredTab = preferredMethod
    ? tabForMethodType(preferredMethod.paymentMethodType)
    : undefined
  const [activeTab, setActiveTab] = useState<'card' | 'directDebit'>(
    preferredTab === 'directDebit' && props.directDebitConnectionId
      ? 'directDebit'
      : preferredTab === 'card' && props.cardConnectionId
        ? 'card'
        : props.cardConnectionId
          ? 'card'
          : 'directDebit'
  )

  // PaymentPanel is not under PortalThemeProvider; it receives the surrounding
  // pane's resolved theme so the Stripe Elements match it exactly.
  const stripeAppearance = useMemo(
    () => buildStripeAppearance(props.themeConfig),
    [props.themeConfig]
  )

  const hasCard = !!props.cardConnectionId
  const hasDirectDebit = !!props.directDebitConnectionId
  const hasBoth =
    hasCard && hasDirectDebit && props.cardConnectionId !== props.directDebitConnectionId

  // Saved methods usable on the active tab. A GoCardless mandate here means we
  // can charge it off-session — no need to send the customer through the hosted
  // flow again (that's only for setting up a *new* mandate).
  const savedMethodsForTab = props.paymentMethods.filter(
    m => tabForMethodType(m.paymentMethodType) === activeTab
  )

  // GoCardless builds its redirect_uri server-side; the desired post-flow page
  // (this URL, minus any stale gocardless_* params) rides along as return_to.
  const activeConnectionId =
    activeTab === 'card' ? props.cardConnectionId : props.directDebitConnectionId

  const returnUrl = useMemo(() => gocardlessReturnUrl(), [])

  // Hosted-checkout direct debit (checkout page, no saved mandate): ONE explicit
  // action that authorises the mandate and pays the first invoice in a single
  // hosted flow. No setup intent is fetched — that pre-created a (mandate-only)
  // Billing Request on every panel render.
  const hostedCheckoutDD =
    activeTab === 'directDebit' &&
    !props.invoiceId &&
    !!props.onHostedDirectDebit &&
    savedMethodsForTab.length === 0

  // Just before the customer leaves for the GoCardless hosted flow, snapshot the
  // invoice's already-failed transactions so the return handler can tell the new
  // charge's failure apart from these — without racing the first post-return poll.
  const stashPreAttempt = useCallback(() => {
    if (props.invoiceId) {
      stashGocardlessPreAttempt(props.invoiceId, props.preAttemptFailedTxIds ?? [])
    }
  }, [props.invoiceId, props.preAttemptFailedTxIds])

  const setupIntentQuery = useQuery(
    setupIntent,
    {
      connectionId: activeConnectionId!,
      connectionType:
        activeTab === 'card' ? ConnectionTypeEnum.CARD : ConnectionTypeEnum.DIRECT_DEBIT,
      returnUrl,
      // Present only on the invoice-payment page: lets GoCardless's return
      // handler charge this invoice once the mandate is set up.
      invoiceId: props.invoiceId,
    },
    { enabled: !!activeConnectionId && !hostedCheckoutDD }
  )

  const intent = setupIntentQuery.data?.setupIntent
  const intentSecret = intent?.intentSecret
  const provider = intent?.provider
  const connectionId = intent?.connectionId

  // The tab bar must stay visible in every state (loading / error / GoCardless
  // hosted redirect / Stripe) so a customer who opened the Direct Debit tab can
  // always get back to Card. Only the panel *body* switches per state.
  const renderBody = () => {
    if (hostedCheckoutDD && props.onHostedDirectDebit && activeConnectionId) {
      return (
        <HostedCheckoutPanel
          totalAmount={props.totalAmount}
          connectionId={activeConnectionId}
          onInitiate={props.onHostedDirectDebit}
        />
      )
    }

    if (setupIntentQuery.isLoading) {
      return <div className="w-full p-6 lg:p-10 text-center">Loading payment options...</div>
    }

    if (setupIntentQuery.isError || !intentSecret || !connectionId || provider === undefined) {
      console.log(
        `setupIntent error: ${
          setupIntentQuery.isError ? setupIntentQuery.error : 'missing intent fields'
        } `
      )
      return (
        <div className="w-full p-6 lg:p-10 text-center text-red-600">
          Unable to initialize payment system. Please try again later.
        </div>
      )
    }

    // GoCardless flow: no embedded SDK. The backend put the BRF
    // `authorisation_url` in `intentSecret` (the field is reused across
    // providers — for Stripe it carries the client_secret instead). The user
    // clicks through to the GoCardless-hosted page; when they come back, the
    // server's return-URL handler upserts the mandate and redirects to this
    // page with a gocardless_status the flow uses to confirm the payment.
    if (provider === ConnectorProviderEnum.GOCARDLESS) {
      // Mandate reuse: if the customer already has a bank account (mandate) on
      // this tab, let them pay it off-session — don't force the hosted flow
      // again. The hosted redirect is only for setting up the *first* (or an
      // additional) mandate. `intentSecret` is the fresh authorisation URL,
      // reused as the "set up a new bank account" link.
      if (savedMethodsForTab.length > 0) {
        return (
          <SavedMandatePanel
            methods={savedMethodsForTab}
            customer={props.customer}
            onPaymentSubmit={props.onPaymentSubmit}
            addNewUrl={intentSecret}
            onBeforeRedirect={stashPreAttempt}
          />
        )
      }
      return (
        <HostedRedirectPanel
          authorisationUrl={intentSecret}
          providerLabel="GoCardless"
          helperText="You'll be redirected to GoCardless to authorise a direct-debit mandate. After you confirm, you'll return here to complete your payment."
          onBeforeRedirect={stashPreAttempt}
        />
      )
    }

    // Stripe flow: mount Elements. publishable_key is in providerPublicKey.
    const stripePublishableKey = intent.providerPublicKey
    if (!stripePublishableKey) {
      return (
        <div className="w-full p-6 lg:p-10 text-center text-red-600">
          Provider configuration is incomplete. Please contact support.
        </div>
      )
    }

    // `key={intentSecret}` remounts Elements per SetupIntent — react-stripe-js
    // ignores clientSecret changes after mount, so a tab switch would otherwise
    // leave Elements bound to the previous intent.
    return (
      <Elements
        key={intentSecret}
        stripe={getStripePromise(stripePublishableKey)}
        options={{
          clientSecret: intentSecret,
          appearance: stripeAppearance,
        }}
      >
        <PaymentPanelInner
          {...props}
          activeConnectionId={connectionId}
          activeConnectionType={activeTab}
        />
      </Elements>
    )
  }

  return (
    <div>
      {/* Tabs for card/direct debit if both are available */}
      {hasBoth && (
        <div className="flex mb-6" style={{ borderBottom: '1px solid var(--mtp-border)' }}>
          <button
            type="button"
            className="flex-1 py-3 px-4 text-sm font-medium transition-colors"
            style={
              activeTab === 'card'
                ? {
                    borderBottom: '2px solid var(--mtp-accent)',
                    color: 'var(--mtp-accent-ink)',
                  }
                : { color: 'var(--mtp-text-2)' }
            }
            onClick={() => setActiveTab('card')}
          >
            <div className="flex items-center justify-center">
              <CreditCard size={16} className="mr-2" />
              Card
            </div>
          </button>
          <button
            type="button"
            className="flex-1 py-3 px-4 text-sm font-medium transition-colors"
            style={
              activeTab === 'directDebit'
                ? {
                    borderBottom: '2px solid var(--mtp-accent)',
                    color: 'var(--mtp-accent-ink)',
                  }
                : { color: 'var(--mtp-text-2)' }
            }
            onClick={() => setActiveTab('directDebit')}
          >
            <div className="flex items-center justify-center">
              <Building size={16} className="mr-2" />
              Direct Debit
            </div>
          </button>
        </div>
      )}

      {renderBody()}
    </div>
  )
}

/**
 * Renders the hosted-redirect branch for providers like GoCardless that
 * collect mandate consent on their own UI rather than via an embedded SDK.
 *
 * Workflow:
 *   1. User clicks "Continue".
 *   2. Browser navigates to the provider's hosted authorisation page.
 *   3. Provider redirects back to our server-side return URL once the
 *      customer consents (or aborts).
 *   4. Our return-URL handler upserts the mandate and bounces the user
 *      back into the portal.
 */
const HostedRedirectPanel: React.FC<{
  authorisationUrl: string
  providerLabel: string
  helperText?: string
  /** Run synchronously before the hosted flow navigates away. */
  onBeforeRedirect?: () => void
}> = ({ authorisationUrl, providerLabel, helperText, onBeforeRedirect }) => {
  return (
    <div className="max-w-md mx-auto p-6">
      <div className="flex items-center gap-3 mb-6">
        <Building size={28} style={{ color: 'var(--mtp-accent)' }} />
        <div>
          <div className="font-medium">Pay by direct debit</div>
          <div className="text-xs text-muted-foreground">Secured by {providerLabel}</div>
        </div>
      </div>
      {helperText && <p className="text-sm text-muted-foreground mb-6">{helperText}</p>}
      <a
        href={authorisationUrl}
        onClick={onBeforeRedirect}
        className="w-full inline-flex items-center justify-center gap-2 py-3 rounded-lg transition-all font-medium hover:opacity-90"
        style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
      >
        <ExternalLink size={16} />
        Continue to {providerLabel}
      </a>
    </div>
  )
}

/**
 * Checkout direct-debit panel (GoCardless, no saved mandate): one explicit
 * action. The click calls the InitiateHostedCheckout RPC, which creates a
 * combined mandate+payment Billing Request and redirects to the hosted
 * authorisation page — mandate authorisation and first payment in one step.
 */
const HostedCheckoutPanel: React.FC<{
  totalAmount: string
  connectionId: string
  onInitiate: (connectionId: string) => Promise<void>
}> = ({ totalAmount, connectionId, onInitiate }) => {
  const [state, setState] = useState<PaymentState>(PaymentState.INITIAL)
  const [error, setError] = useState<string | null>(null)

  const handleClick = async () => {
    setState(PaymentState.PROCESSING)
    setError(null)
    try {
      // On success this redirects to the hosted page and never resolves.
      await onInitiate(connectionId)
    } catch (err) {
      setError(
        err instanceof Error && err.message
          ? err.message
          : 'Unable to start the direct debit payment. Please try again.'
      )
      setState(PaymentState.ERROR)
    }
  }

  return (
    <div className="max-w-md mx-auto p-6">
      <div className="flex items-center gap-3 mb-6">
        <Building size={28} style={{ color: 'var(--mtp-accent)' }} />
        <div>
          <div className="font-medium">Pay by direct debit</div>
          <div className="text-xs text-muted-foreground">Secured by GoCardless</div>
        </div>
      </div>
      <p className="text-sm text-muted-foreground mb-6">
        You&apos;ll be securely redirected to set up your bank details and pay {totalAmount}.
      </p>

      {error && (
        <div
          className="mb-4 p-3 rounded-lg text-sm flex items-start"
          style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
        >
          <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <button
        type="button"
        onClick={handleClick}
        disabled={state === PaymentState.PROCESSING}
        className="w-full inline-flex items-center justify-center gap-2 py-3 rounded-lg transition-all font-medium disabled:cursor-not-allowed disabled:opacity-60 hover:opacity-90"
        style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
      >
        {state === PaymentState.PROCESSING ? (
          <>
            <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
            Redirecting to GoCardless...
          </>
        ) : (
          <>
            <ExternalLink size={16} />
            Pay {totalAmount} by direct debit
          </>
        )}
      </button>
    </div>
  )
}
