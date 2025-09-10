import { useMutation } from '@connectrpc/connect-query'
import { ArrowLeft } from 'lucide-react'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { confirmCheckout } from '@/rpc/portal/checkout/v1/checkout-PortalCheckoutService_connectquery'
import { formatCurrency } from '@/utils/numbers'

import { BillingInfo } from './components/BillingInfo'
import { SubscriptionSummary } from './components/SubscriptionSummary'
import { CheckoutFlowProps } from './types'
/**
 * Main checkout flow component
 */
const CheckoutFlow: React.FC<CheckoutFlowProps> = ({ checkoutData }) => {
  const [isAddressEditing, setIsAddressEditing] = useState(false)
  const navigate = useNavigate()
  const { subscription, customer, paymentMethods, totalAmount } = checkoutData

  // Mutation to confirm the checkout
  const confirmCheckoutMutation = useMutation(confirmCheckout, {
    onError: error => {
      console.error('Checkout confirmation error:', error)
    },
  })

  // Mutation to add a new payment method

  /**
   * Process payment with selected payment method
   */
  const handlePaymentSubmit = async (paymentMethodId: string) => {
    try {
      if (!subscription?.subscription?.currency) {
        throw new Error('Currency is not defined')
      }

      await confirmCheckoutMutation.mutateAsync({
        displayedAmount: totalAmount,
        displayedCurrency: subscription.subscription.currency,
        paymentMethodId,
      })

      // On success, redirect to success page
      const params = new URLSearchParams({
        plan: subscription.subscription.planName || '',
        customer: customer?.name || '',
      })
      navigate(`success?${params.toString()}`)
    } catch (error) {
      console.error('Payment submission error:', error)
      throw error // Let the PaymentPanel handle this error
    }
  }

  if (!subscription?.subscription || !customer) {
    return <div className="p-8 text-center">Loading checkout information...</div>
  }

  return (
    <div className="flex flex-col lg:flex-row min-h-screen">
      {/* Mobile header */}
      <div className="lg:hidden w-full p-4 border-b border-gray-100 flex items-center">
        <button className=" flex items-center" onClick={() => window.history.back()}>
          <ArrowLeft size={16} className="mr-2" />
          <span className="mr-2">{checkoutData.tradeName}</span>
          {checkoutData.logoUrl && (
            <img src={checkoutData.logoUrl} alt="logo" width={24} height={24} />
          )}
        </button>
        <div className="text-sm font-medium mx-auto">
          Subscribe to {subscription.subscription.planName}
        </div>
      </div>

      {/* Main content */}
      <div className="min-h-screen max-h-screen w-full flex md:flex-row flex-col overflow-auto">
        {/* Left panel - Order summary */}
        <div className="flex flex-col md:h-screen bg-background-gray gap-5 px-5 md:px-4 lg:px-20 lg:pt-16 lg:pb-20 pt-5 pb-5 border-b border-border-regular md:pb-8 md:pt-16 w-full md:overflow-auto">
          <div className="md:max-w-[500px] w-full ml-auto  ">
            <SubscriptionSummary checkoutData={checkoutData} />
          </div>
        </div>
        {/* Right panel - Payment form */}
        <div className="w-full flex lg:px-20 md:px-4 px-5 flex-col bg-white md:h-screen md:overflow-auto lg:pt-16 py-5 shadow-md">
          <div className="mr-auto ml-auto md:ml-0 md:pt-0 md:h-screen w-full max-w-[440px]">
            {/* Billing information */}
            <BillingInfo
              customer={customer}
              isEditing={isAddressEditing}
              setIsEditing={setIsAddressEditing}
            />

            {/* Payment panel */}
            <PaymentPanel
              customer={customer}
              paymentMethods={paymentMethods || []}
              currency={subscription.subscription.currency}
              totalAmount={formatCurrency(totalAmount, subscription.subscription.currency)}
              onPaymentSubmit={handlePaymentSubmit}
              cardConnectionId={subscription.subscription.cardConnectionId}
              directDebitConnectionId={subscription.subscription.directDebitConnectionId}
            />
          </div>
        </div>
      </div>
    </div>
  )
}

export default CheckoutFlow
