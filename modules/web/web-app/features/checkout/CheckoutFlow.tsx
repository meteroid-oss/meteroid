import React, { useState } from 'react'

import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { ArrowLeft } from 'lucide-react'
import { BillingInfo } from './components/BillingInfo'
import { SubscriptionSummary } from './components/SubscriptionSummary'
import { CheckoutFlowProps } from './types'

/**
 * Main checkout flow component
 */
const CheckoutFlow: React.FC<CheckoutFlowProps> = ({ checkoutData }) => {
  const [isAddressEditing, setIsAddressEditing] = useState(false)
  const { subscription, customer, paymentMethods } = checkoutData

  /**
   * Process payment with selected payment method
   */
  const handlePaymentSubmit = async (paymentMethodId: string, isNew: boolean) => {
    try {
      // Determine the API endpoint based on whether we're using a saved or new payment method
      const endpoint = isNew
        ? '/api/checkout/process-payment-with-new-method'
        : '/api/checkout/process-payment'

      const response = await fetch(endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          paymentMethodId,
          customerId: customer?.id,
          subscriptionId: subscription?.subscription?.id,
        }),
      })

      if (!response.ok) {
        const errorData = await response.json()
        throw new Error(errorData.message || 'Payment failed')
      }

      // Check if we need to redirect (e.g., for 3D Secure or bank redirects)
      const data = await response.json()
      if (data.redirectUrl) {
        window.location.href = data.redirectUrl
        return
      }

      // On success, redirect to success page
      window.location.href = `/checkout/success?subscription=${subscription?.subscription?.id}`
    } catch (error) {
      console.error('Payment submission error:', error)
      throw error // Let the PaymentPanel handle this error
    }
  }

  /**
   * Update billing address
   */
  const handleSaveAddress = async (address: any) => {
    try {
      const response = await fetch('/api/customers/update-billing-address', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          customerId: customer?.id,
          billingAddress: address,
        }),
      })

      if (!response.ok) {
        throw new Error('Failed to update billing address')
      }

      setIsAddressEditing(false)
    } catch (error) {
      console.error('Error updating billing address:', error)
      // You could set an error state here to display to the user
    }
  }

  if (!subscription || !customer) {
    return <div className="p-8 text-center">Loading checkout information...</div>
  }

  // Format the total amount for display
  const formatTotalAmount = () => {
    if (!subscription.totalAmount || !subscription.subscription?.currency) {
      return 'N/A'
    }

    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: subscription.subscription.currency,
    }).format(subscription.totalAmount / 100) // Convert cents to dollars
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
          Subscribe to {subscription.subscription?.planName}
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
              onEdit={() => setIsAddressEditing(true)}
              onSave={handleSaveAddress}
              onCancel={() => setIsAddressEditing(false)}
            />

            {/* Payment panel */}
            <PaymentPanel
              customer={customer}
              paymentMethods={paymentMethods || []}
              currency={subscription.subscription?.currency || 'USD'}
              totalAmount={formatTotalAmount()}
              onPaymentSubmit={handlePaymentSubmit}
            />
          </div>
        </div>
      </div>
    </div>
  )
}

export default CheckoutFlow
